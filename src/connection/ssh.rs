use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use tokio::sync::Mutex;
use tokio::time::timeout;

use super::TabletConnection;

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
}

impl SshConnection {
    pub async fn connect(host: &str, port: u16, opts: &ConnectOptions) -> anyhow::Result<Self> {
        let addr = format!("{host}:{port}");
        let config = Arc::new(client::Config::default());
        let mut handle = timeout(
            opts.timeout,
            client::connect(config, addr.as_str(), ClientHandler),
        )
        .await
        .map_err(|_| {
            crate::error::CliError::ConnectionFailed(format!(
                "SSH connect to {addr} timed out after {:?}",
                opts.timeout
            ))
        })?
        .map_err(|e| {
            crate::error::CliError::ConnectionFailed(format!("SSH connect to {addr} failed: {e}"))
        })?;

        if !authenticate(&mut handle, opts).await? {
            return Err(crate::error::CliError::AuthFailed(format!(
                "all auth methods failed for {}@{addr}",
                opts.user
            ))
            .into());
        }

        let channel = handle
            .channel_open_session()
            .await
            .context("open sftp channel")?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .context("request sftp subsystem")?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .context("start sftp session")?;

        Ok(Self {
            handle: Mutex::new(handle),
            sftp,
        })
    }

    pub async fn disconnect(&self) {
        let handle = self.handle.lock().await;
        handle
            .disconnect(Disconnect::ByApplication, "bye", "en")
            .await
            .ok();
    }
}

async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    opts: &ConnectOptions,
) -> anyhow::Result<bool> {
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        verbose(opts, &format!("auth: trying ssh-agent ({sock})"));
        if try_agent(handle, &opts.user).await.unwrap_or(false) {
            verbose(opts, "auth: ssh-agent accepted");
            return Ok(true);
        }
    }

    if let Some(kf) = &opts.key_file {
        let expanded = expand_tilde(kf);
        if expanded.exists() {
            verbose(opts, &format!("auth: trying key {}", expanded.display()));
            if try_key_file(handle, &opts.user, &expanded)
                .await
                .unwrap_or(false)
            {
                verbose(opts, "auth: key file accepted");
                return Ok(true);
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
            return Ok(true);
        }
    }

    Ok(false)
}

async fn try_agent(handle: &mut Handle<ClientHandler>, user: &str) -> anyhow::Result<bool> {
    use russh::keys::agent::client::AgentClient;
    let sock = std::env::var("SSH_AUTH_SOCK").map_err(|_| anyhow!("no SSH_AUTH_SOCK"))?;
    let mut agent = AgentClient::connect_uds(sock)
        .await
        .context("connect agent")?;
    let identities = agent
        .request_identities()
        .await
        .context("list identities")?;
    for pubkey in identities {
        let auth = handle
            .authenticate_publickey_with(user, pubkey, Some(HashAlg::Sha512), &mut agent)
            .await;
        if let Ok(r) = auth
            && r.success()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn try_key_file(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    path: &Path,
) -> anyhow::Result<bool> {
    let key = load_secret_key(path, None).with_context(|| format!("load {}", path.display()))?;
    let with_hash = PrivateKeyWithHashAlg::new(Arc::new(key), Some(HashAlg::Sha512));
    let auth = handle.authenticate_publickey(user, with_hash).await?;
    Ok(auth.success())
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

impl TabletConnection for SshConnection {
    async fn read_file(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        self.sftp
            .read(path)
            .await
            .with_context(|| format!("sftp read {path}"))
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> anyhow::Result<()> {
        self.sftp
            .write(path, data)
            .await
            .with_context(|| format!("sftp write {path}"))
    }

    async fn list_dir(&self, path: &str) -> anyhow::Result<Vec<String>> {
        let dir = self
            .sftp
            .read_dir(path)
            .await
            .with_context(|| format!("sftp read_dir {path}"))?;
        let mut out = Vec::new();
        for entry in dir {
            let name = entry.file_name();
            if name != "." && name != ".." {
                out.push(name);
            }
        }
        Ok(out)
    }

    async fn remove_file(&self, path: &str) -> anyhow::Result<()> {
        self.sftp
            .remove_file(path)
            .await
            .with_context(|| format!("sftp remove_file {path}"))
    }

    async fn execute(&self, command: &str) -> anyhow::Result<String> {
        let handle = self.handle.lock().await;
        let mut channel = handle
            .channel_open_session()
            .await
            .with_context(|| format!("open channel for `{command}`"))?;
        channel
            .exec(true, command)
            .await
            .with_context(|| format!("exec `{command}`"))?;
        let mut buf = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => buf.extend_from_slice(data),
                ChannelMsg::ExtendedData { .. } => {}
                ChannelMsg::ExitStatus { .. } | ChannelMsg::Eof => break,
                _ => {}
            }
        }
        String::from_utf8(buf).context("command output not UTF-8")
    }

    async fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        self.sftp
            .try_exists(path)
            .await
            .map_err(|e| anyhow::Error::new(e).context(format!("sftp stat {path}")))
    }
}
