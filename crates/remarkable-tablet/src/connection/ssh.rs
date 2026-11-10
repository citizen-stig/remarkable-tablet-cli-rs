use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use russh_sftp::client::fs::Metadata;
use russh_sftp::protocol::FileType as SftpFileType;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;

use super::{RemoteEntry, RemoteFileKind, RemoteMetadata, TabletConnection};
use crate::error::{IoSource, TabletError};

pub struct ConnectOptions {
    pub user: String,
    pub password: Option<String>,
    pub key_file: Option<PathBuf>,
    pub timeout: Duration,
    pub verbose: bool,
}

struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub struct SshConnection {
    handle: Mutex<Handle<ClientHandler>>,
    /// `russh-sftp`'s `SftpSession` multiplexes requests internally via SFTP
    /// request IDs and exposes `&self` methods, so concurrent `read`/`list_dir`
    /// calls are safe and allow many in-flight requests on a single channel.
    /// We keep no mutex here — that's the whole point of dropping the lock,
    /// since holding one across `await` was serializing every SFTP round-trip.
    sftp: SftpSession,
    verbose: bool,
}

impl SshConnection {
    /// # Errors
    /// Returns [`TabletError::ConnectTimeout`] / [`TabletError::Connect`] on
    /// TCP/DNS failure or connect timeout, [`TabletError::AuthFailed`] if no
    /// auth method succeeds, or [`TabletError::Ssh`] if the SFTP subsystem
    /// cannot be opened.
    pub async fn connect(
        host: &str,
        port: u16,
        opts: &ConnectOptions,
    ) -> Result<Self, TabletError> {
        let addr = format!("{host}:{port}");
        let config = Arc::new(client::Config::default());
        let connect = async {
            let mut addresses: Vec<_> = tokio::net::lookup_host(addr.as_str())
                .await
                .map_err(russh::Error::from)?
                .collect();
            addresses.sort_by_key(std::net::SocketAddr::is_ipv6);
            client::connect(config, addresses.as_slice(), ClientHandler).await
        };
        let mut handle = timeout(opts.timeout, connect)
            .await
            .map_err(|_| TabletError::ConnectTimeout {
                addr: addr.clone(),
                timeout: opts.timeout,
            })?
            .map_err(|source| TabletError::Connect {
                addr: addr.clone(),
                source,
            })?;

        if !authenticate(&mut handle, opts).await {
            return Err(TabletError::AuthFailed(format!(
                "all auth methods failed for {}@{addr}",
                opts.user
            )));
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|source| TabletError::Ssh {
                op: "open sftp channel".to_string(),
                source,
            })?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|source| TabletError::Ssh {
                op: "request sftp subsystem".to_string(),
                source,
            })?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|source| TabletError::Io {
                op: "start_session",
                path: String::new(),
                source: IoSource::Sftp(source),
            })?;

        Ok(Self {
            handle: Mutex::new(handle),
            sftp,
            verbose: opts.verbose,
        })
    }

    /// Send a best-effort SSH `disconnect`. Errors are logged under
    /// `--verbose` and otherwise ignored — the session is being torn down
    /// regardless and there's no useful caller action when "bye" can't be
    /// delivered (the remote may have already closed the channel).
    pub async fn disconnect(&self) {
        let handle = self.handle.lock().await;
        if let Err(err) = handle
            .disconnect(Disconnect::ByApplication, "bye", "en")
            .await
            && self.verbose
        {
            eprintln!("ssh disconnect: {err}");
        }
    }
}

/// Try every configured auth method in priority order; return `true` on the
/// first one that succeeds. Each method swallows its own errors — there's no
/// useful caller action between methods, and the caller distinguishes
/// "authenticated" from "every method failed" via the bool, not via an error.
async fn authenticate(handle: &mut Handle<ClientHandler>, opts: &ConnectOptions) -> bool {
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        verbose(opts, &format!("auth: trying ssh-agent ({sock})"));
        if try_agent(handle, &opts.user).await {
            verbose(opts, "auth: ssh-agent accepted");
            return true;
        }
    }

    if let Some(kf) = &opts.key_file {
        let expanded = expand_tilde(kf);
        if expanded.exists() {
            verbose(opts, &format!("auth: trying key {}", expanded.display()));
            if try_key_file(handle, &opts.user, &expanded).await {
                verbose(opts, "auth: key file accepted");
                return true;
            }
        }
    }

    if let Some(pw) = &opts.password {
        verbose(opts, "auth: trying password");
        if handle
            .authenticate_password(&opts.user, pw.clone())
            .await
            .is_ok_and(|r| r.success())
        {
            verbose(opts, "auth: password accepted");
            return true;
        }
    }

    false
}

async fn try_agent(handle: &mut Handle<ClientHandler>, user: &str) -> bool {
    use russh::keys::agent::client::AgentClient;
    let Ok(sock) = std::env::var("SSH_AUTH_SOCK") else {
        return false;
    };
    let Ok(mut agent) = AgentClient::connect_uds(sock).await else {
        return false;
    };
    let Ok(identities) = agent.request_identities().await else {
        return false;
    };
    for pubkey in identities {
        if let Ok(r) = handle
            .authenticate_publickey_with(user, pubkey, Some(HashAlg::Sha512), &mut agent)
            .await
            && r.success()
        {
            return true;
        }
    }
    false
}

async fn try_key_file(handle: &mut Handle<ClientHandler>, user: &str, path: &Path) -> bool {
    let Ok(key) = load_secret_key(path, None) else {
        return false;
    };
    let with_hash = PrivateKeyWithHashAlg::new(Arc::new(key), Some(HashAlg::Sha512));
    let Ok(auth) = handle.authenticate_publickey(user, with_hash).await else {
        return false;
    };
    auth.success()
}

fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

fn verbose(opts: &ConnectOptions, msg: &str) {
    if opts.verbose {
        eprintln!("{msg}");
    }
}

fn into_remote_metadata(meta: &Metadata) -> RemoteMetadata {
    RemoteMetadata {
        size: meta.size,
        mtime: meta
            .mtime
            .map(|secs| UNIX_EPOCH + Duration::from_secs(u64::from(secs))),
        kind: match meta.file_type() {
            SftpFileType::Dir => RemoteFileKind::Dir,
            SftpFileType::File => RemoteFileKind::File,
            SftpFileType::Symlink | SftpFileType::Other => RemoteFileKind::Other,
        },
    }
}

impl SshConnection {
    /// Run `command` and return raw stdout, for binary output such as a tar stream.
    async fn exec_bytes(&self, command: &str) -> Result<Vec<u8>, TabletError> {
        let handle = self.handle.lock().await;
        let mut channel =
            handle
                .channel_open_session()
                .await
                .map_err(|source| TabletError::Ssh {
                    op: format!("open channel for `{command}`"),
                    source,
                })?;
        channel
            .exec(true, command)
            .await
            .map_err(|source| TabletError::Ssh {
                op: format!("exec `{command}`"),
                source,
            })?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        let mut exit_signal = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
                ChannelMsg::ExtendedData { ref data, .. } => stderr.extend_from_slice(data),
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => exit_status = Some(status),
                ChannelMsg::ExitSignal {
                    signal_name,
                    core_dumped,
                    error_message,
                    ..
                } => {
                    exit_signal = Some((format!("{signal_name:?}"), core_dumped, error_message));
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        if let Some((signal_name, core_dumped, message)) = exit_signal {
            return Err(TabletError::CommandTerminated {
                command: command.to_string(),
                signal_name,
                core_dumped,
                message,
            });
        }
        if let Some(exit_status) = exit_status
            && exit_status != 0
        {
            return Err(TabletError::CommandFailed {
                command: command.to_string(),
                exit_status,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            });
        }
        Ok(stdout)
    }
}

/// Minimal reader for the ustar archives `BusyBox` `tar -c` writes: regular
/// files only, names up to 100 bytes, no compression. Returns
/// `(basename without "./", contents)` pairs.
fn parse_ustar(archive: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    const BLOCK: usize = 512;
    let mut files = Vec::new();
    let mut offset = 0;
    while offset + BLOCK <= archive.len() {
        let header = &archive[offset..offset + BLOCK];
        if header.iter().all(|&b| b == 0) {
            return Ok(files);
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        };
        let name = field(0..100);
        let size_field = field(124..136);
        let size = usize::from_str_radix(size_field.trim(), 8).map_err(|err| {
            format!("tar header at offset {offset} for {name:?}: bad size {size_field:?}: {err}")
        })?;
        let data_start = offset + BLOCK;
        let data = archive.get(data_start..data_start + size).ok_or_else(|| {
            format!(
                "tar entry {name:?} at offset {offset} claims {size} bytes, archive has {}",
                archive.len().saturating_sub(data_start)
            )
        })?;
        if matches!(header[156], b'0' | 0) {
            let name = name.strip_prefix("./").unwrap_or(&name).to_string();
            files.push((name, data.to_vec()));
        }
        offset = data_start + size.div_ceil(BLOCK) * BLOCK;
    }
    if offset == archive.len() {
        return Ok(files);
    }
    Err(format!(
        "tar stream truncated: {} trailing bytes at offset {offset} of {}",
        archive.len() - offset,
        archive.len()
    ))
}

impl TabletConnection for SshConnection {
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, TabletError> {
        let mut file = self
            .sftp
            .open(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "open",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })?;
        // russh-sftp sends one READ per poll sized to spare capacity; starting
        // empty makes tokio grow it 32, 64, 128... = one round-trip per step.
        let mut data = Vec::with_capacity(256 * 1024);
        file.read_to_end(&mut data)
            .await
            .map_err(|source| TabletError::Io {
                op: "read",
                path: path.to_string(),
                source: IoSource::Local(source),
            })?;
        file.shutdown().await.map_err(|source| TabletError::Io {
            op: "close",
            path: path.to_string(),
            source: IoSource::Local(source),
        })?;
        Ok(data)
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), TabletError> {
        // `Sftp::write` opens with `OpenFlags::WRITE` only — missing `CREATE`,
        // so it fails on new files with "No such file". `create` opens with
        // `CREATE | TRUNCATE | WRITE`, which is what we want for both new
        // writes and overwrites.
        let mut file = self
            .sftp
            .create(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "create",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })?;
        // `write_all` and `shutdown` are tokio AsyncWrite methods returning
        // `io::Error` even though the transport is SFTP. Surface as
        // `IoSource::Local` to preserve `io::ErrorKind` (the Sftp variant
        // would lose it via russh_sftp's `Error::IO(String)` conversion).
        file.write_all(data)
            .await
            .map_err(|source| TabletError::Io {
                op: "write",
                path: path.to_string(),
                source: IoSource::Local(source),
            })?;
        file.shutdown().await.map_err(|source| TabletError::Io {
            op: "close",
            path: path.to_string(),
            source: IoSource::Local(source),
        })?;
        Ok(())
    }

    async fn read_dir(&self, path: &str) -> Result<Vec<RemoteEntry>, TabletError> {
        let dir = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "read_dir",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })?;
        Ok(dir
            .map(|entry| RemoteEntry {
                name: entry.file_name(),
                metadata: into_remote_metadata(&entry.metadata()),
            })
            .collect())
    }

    async fn stat(&self, path: &str) -> Result<RemoteMetadata, TabletError> {
        let meta = self
            .sftp
            .metadata(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "stat",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })?;
        Ok(into_remote_metadata(&meta))
    }

    async fn remove_file(&self, path: &str) -> Result<(), TabletError> {
        self.sftp
            .remove_file(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "remove_file",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })
    }

    async fn remove_dir_all(&self, path: &str) -> Result<(), TabletError> {
        // SFTP exposes only non-recursive `remove_dir`. Falling back to a
        // single shell `rm -rf` keeps this O(1) round-trips and matches
        // `rm -rf`'s "no-op on missing" contract. `--` guards against any
        // path starting with `-` (UUIDs never do, but be safe).
        let escaped = crate::tablet::shell_single_quote(path);
        self.execute(&format!("rm -rf -- {escaped}")).await?;
        Ok(())
    }

    async fn execute(&self, command: &str) -> Result<String, TabletError> {
        String::from_utf8(self.exec_bytes(command).await?)
            .map_err(|source| TabletError::CommandOutputNotUtf8 { source })
    }

    async fn read_metadata_files(
        &self,
        dir: &str,
    ) -> Result<HashMap<String, Result<Vec<u8>, TabletError>>, TabletError> {
        // One tar stream instead of an SFTP open/read/close per file: ~900
        // files took ~0.9 s over SFTP vs ~0.3 s as tar on USB. The guard
        // exists because BusyBox tar fails with "empty archive" on no input.
        let dir = crate::tablet::shell_single_quote(dir);
        let command = format!(
            "cd {dir} || exit 1; [ -z \"$(find . -maxdepth 1 -name '*.metadata')\" ] || \
             find . -maxdepth 1 \\( -name '*.metadata' -o -name '*.content' \\) | tar -cf - -T -"
        );
        let archive = self.exec_bytes(&command).await.map_err(|err| match err {
            // stdout is a partial tar stream: megabytes of noise in an error message.
            TabletError::CommandFailed {
                command,
                exit_status,
                stdout,
                stderr,
            } => TabletError::CommandFailed {
                command,
                exit_status,
                stdout: format!("<{} bytes of tar output omitted>", stdout.len()),
                stderr,
            },
            other => other,
        })?;
        let files = parse_ustar(&archive)
            .map_err(|message| TabletError::CommandOutput { command, message })?;
        Ok(files
            .into_iter()
            .map(|(name, data)| (name, Ok(data)))
            .collect())
    }

    async fn file_exists(&self, path: &str) -> Result<bool, TabletError> {
        self.sftp
            .try_exists(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "exists",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::parse_ustar;

    fn ustar_entry(name: &str, data: &[u8]) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let size = format!("{:011o}\0", data.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[156] = b'0';
        let mut out = header;
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
        out
    }

    #[test]
    fn parses_two_files_and_strips_dot_slash() {
        let big = vec![b'x'; 700];
        let mut archive = ustar_entry("./a.metadata", b"{}");
        archive.extend(ustar_entry("b.content", &big));
        archive.extend(vec![0u8; 1024]);

        let files = parse_ustar(&archive).unwrap();
        assert_eq!(
            files,
            vec![
                ("a.metadata".to_string(), b"{}".to_vec()),
                ("b.content".to_string(), big),
            ]
        );
    }

    #[test]
    fn empty_input_is_empty_listing() {
        assert_eq!(parse_ustar(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn truncated_entry_reports_claimed_and_actual_size() {
        let mut archive = ustar_entry("a.content", &[b'x'; 600]);
        archive.truncate(512 + 100);
        let err = parse_ustar(&archive).unwrap_err();
        assert!(err.contains("claims 600 bytes, archive has 100"), "{err}");
    }
}
