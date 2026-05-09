use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use russh_sftp::client::fs::Metadata;
use russh_sftp::protocol::FileType as SftpFileType;
use tokio::io::AsyncWriteExt;
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
        let mut handle = timeout(
            opts.timeout,
            client::connect(config, addr.as_str(), ClientHandler),
        )
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
            .map(|r| r.success())
            .unwrap_or(false)
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

impl TabletConnection for SshConnection {
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, TabletError> {
        self.sftp
            .read(path)
            .await
            .map_err(|source| TabletError::Io {
                op: "read",
                path: path.to_string(),
                source: IoSource::Sftp(source),
            })
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
        let mut buf = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => buf.extend_from_slice(data),
                ChannelMsg::ExitStatus { .. } | ChannelMsg::Eof => break,
                _ => {}
            }
        }
        String::from_utf8(buf).map_err(|source| TabletError::CommandOutputNotUtf8 { source })
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
