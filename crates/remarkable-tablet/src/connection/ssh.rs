use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key};
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
        _server_public_key: &PublicKeyOrCertificate,
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
    /// Opened on first use: tree loads go through `exec` and never need it.
    sftp: tokio::sync::OnceCell<SftpSession>,
    verbose: bool,
}

impl SshConnection {
    /// # Errors
    /// Returns [`TabletError::ConnectTimeout`] / [`TabletError::Connect`] on
    /// TCP/DNS failure or connect timeout, or [`TabletError::AuthFailed`] if no
    /// auth method succeeds.
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

        Ok(Self {
            handle: Mutex::new(handle),
            sftp: tokio::sync::OnceCell::new(),
            verbose: opts.verbose,
        })
    }

    /// # Errors
    /// Returns [`TabletError::Ssh`] / [`TabletError::Io`] if the SFTP
    /// subsystem cannot be opened.
    async fn sftp(&self) -> Result<&SftpSession, TabletError> {
        self.sftp
            .get_or_try_init(|| async {
                let channel = self
                    .handle
                    .lock()
                    .await
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
                SftpSession::new(channel.into_stream())
                    .await
                    .map_err(|source| TabletError::Io {
                        op: "start_session",
                        path: String::new(),
                        source: IoSource::Sftp(source),
                    })
            })
            .await
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
    for identity in identities {
        if let Ok(r) = handle
            .authenticate_publickey_with(
                user,
                identity.public_key().into_owned(),
                Some(HashAlg::Sha512),
                &mut agent,
            )
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

/// Splits the output of `BusyBox` `tail -v -c +1 FILE...` into
/// `(name, contents)` pairs. Each file is preceded by `==> NAME <==\n`, and
/// every header after the first by an extra `\n`. The split is unambiguous for
/// JSON files: a raw newline followed by `==> ` never occurs in valid JSON.
fn parse_tail(stream: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    const SEP: &[u8] = b"\n==> ";
    const NAME_END: &[u8] = b" <==\n";
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    if stream.is_empty() {
        return Ok(Vec::new());
    }
    let mut rest = stream.strip_prefix(&SEP[1..]).ok_or_else(|| {
        format!(
            "expected `==> ` header, got {:?}",
            String::from_utf8_lossy(&stream[..stream.len().min(40)])
        )
    })?;
    let mut files = Vec::new();
    loop {
        let name_len = find(rest, NAME_END).ok_or_else(|| {
            format!(
                "unterminated header after {} files: {:?}",
                files.len(),
                String::from_utf8_lossy(&rest[..rest.len().min(80)])
            )
        })?;
        let name = String::from_utf8_lossy(&rest[..name_len]).into_owned();
        rest = &rest[name_len + NAME_END.len()..];
        let Some(len) = find(rest, SEP) else {
            files.push((name, rest.to_vec()));
            return Ok(files);
        };
        files.push((name, rest[..len].to_vec()));
        rest = &rest[len + SEP.len()..];
    }
}

impl TabletConnection for SshConnection {
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, TabletError> {
        let mut file = self
            .sftp()
            .await?
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
            .sftp()
            .await?
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
            .sftp()
            .await?
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
            .sftp()
            .await?
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
        self.sftp()
            .await?
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
        // One stream instead of an SFTP open/read/close per file: ~900 files
        // took ~0.9 s over SFTP vs ~0.3 s streamed on USB. `tail` headers are
        // ~60 bytes per file vs tar's 512 + padding, 14% fewer bytes, and the
        // tablet's cipher throughput is the bottleneck. Unmatched globs stay
        // literal in sh, hence the guards; each glob of the ~2700-entry
        // directory costs the tablet ~10 ms, so each pattern is expanded once.
        let dir = crate::tablet::shell_single_quote(dir);
        let command = format!(
            "cd {dir} || exit 1; set -- *.content; [ -e \"$1\" ] || shift; \
             set -- *.metadata \"$@\"; [ -e \"$1\" ] || exit 0; \
             exec tail -v -c +1 \"$@\""
        );
        let archive = self.exec_bytes(&command).await.map_err(|err| match err {
            // stdout is a partial stream: megabytes of noise in an error message.
            TabletError::CommandFailed {
                command,
                exit_status,
                stdout,
                stderr,
            } => TabletError::CommandFailed {
                command,
                exit_status,
                stdout: format!("<{} bytes of tail output omitted>", stdout.len()),
                stderr,
            },
            other => other,
        })?;
        let files = parse_tail(&archive)
            .map_err(|message| TabletError::CommandOutput { command, message })?;
        Ok(files
            .into_iter()
            .map(|(name, data)| (name, Ok(data)))
            .collect())
    }

    async fn file_exists(&self, path: &str) -> Result<bool, TabletError> {
        self.sftp()
            .await?
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
    use super::parse_tail;

    #[test]
    fn parses_files_with_and_without_trailing_newline_and_empty() {
        let stream = b"==> a.metadata <==\n{\n}\n\n==> b.content <==\n{}\n==> c.content <==\n";
        assert_eq!(
            parse_tail(stream).unwrap(),
            vec![
                ("a.metadata".to_string(), b"{\n}\n".to_vec()),
                ("b.content".to_string(), b"{}".to_vec()),
                ("c.content".to_string(), Vec::new()),
            ]
        );
    }

    #[test]
    fn empty_input_is_empty_listing() {
        assert_eq!(parse_tail(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn missing_header_reports_what_was_received() {
        let err = parse_tail(b"tail: can't open").unwrap_err();
        assert!(err.contains("tail: can't open"), "{err}");
    }
}
