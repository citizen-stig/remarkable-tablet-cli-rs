use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio::time::timeout;

pub trait TabletConnection: Send + Sync {
    fn read_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;
    // TODO: for larger files we might prefer a buffer or stream; revisit when
    // upload/download commands land.
    fn write_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
        data: &[u8],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn list_dir<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
    fn remove_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn execute(&self, command: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
    fn file_exists<P: AsRef<Path> + Send>(
        &self,
        path: P,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

// ---------- Production: SSH ----------

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
    sftp: Mutex<SftpSession>,
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
            sftp: Mutex::new(sftp),
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
    let mut agent = AgentClient::connect_uds(sock).await.context("connect agent")?;
    let identities = agent.request_identities().await.context("list identities")?;
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

fn path_to_string<P: AsRef<Path>>(path: P) -> anyhow::Result<String> {
    path.as_ref()
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("non-UTF-8 path: {}", path.as_ref().display()))
}

impl TabletConnection for SshConnection {
    async fn read_file<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<Vec<u8>> {
        let s = path_to_string(&path)?;
        let sftp = self.sftp.lock().await;
        sftp.read(s)
            .await
            .with_context(|| format!("sftp read {}", path.as_ref().display()))
    }

    async fn write_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let s = path_to_string(&path)?;
        let sftp = self.sftp.lock().await;
        sftp.write(s, data)
            .await
            .with_context(|| format!("sftp write {}", path.as_ref().display()))
    }

    async fn list_dir<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<Vec<String>> {
        let s = path_to_string(&path)?;
        let sftp = self.sftp.lock().await;
        let dir = sftp
            .read_dir(s)
            .await
            .with_context(|| format!("sftp read_dir {}", path.as_ref().display()))?;
        let mut out = Vec::new();
        for entry in dir {
            let name = entry.file_name();
            if name != "." && name != ".." {
                out.push(name);
            }
        }
        Ok(out)
    }

    async fn remove_file<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<()> {
        let s = path_to_string(&path)?;
        let sftp = self.sftp.lock().await;
        sftp.remove_file(s)
            .await
            .with_context(|| format!("sftp remove_file {}", path.as_ref().display()))
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

    async fn file_exists<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<bool> {
        let s = path_to_string(&path)?;
        let sftp = self.sftp.lock().await;
        sftp.try_exists(s)
            .await
            .map_err(|e| anyhow::Error::new(e).context(format!("sftp stat {}", path.as_ref().display())))
    }
}

// ---------- Test double: FakeConnection ----------

pub struct FakeConnection {
    root: TempDir,
    commands: std::sync::Mutex<Vec<(String, String)>>,
}

impl FakeConnection {
    pub fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("tempdir"),
            commands: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn local<P: AsRef<Path>>(&self, remote: P) -> PathBuf {
        let s = remote.as_ref().to_string_lossy();
        let rel = s.trim_start_matches('/');
        self.root.path().join(rel)
    }

    pub fn set_file<P: AsRef<Path>>(&self, path: P, data: impl AsRef<[u8]>) {
        let p = self.local(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, data.as_ref()).unwrap();
    }

    pub fn mkdir<P: AsRef<Path>>(&self, path: P) {
        std::fs::create_dir_all(self.local(path)).unwrap();
    }

    pub fn set_command_output(&self, cmd_substring: &str, output: &str) {
        let mut cmds = self.commands.lock().unwrap();
        if let Some(entry) = cmds.iter_mut().find(|(s, _)| s == cmd_substring) {
            entry.1 = output.to_string();
        } else {
            cmds.push((cmd_substring.to_string(), output.to_string()));
        }
    }
}

impl Default for FakeConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl TabletConnection for FakeConnection {
    async fn read_file<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<Vec<u8>> {
        let p = self.local(&path);
        std::fs::read(&p).with_context(|| format!("fake read_file {}", p.display()))
    }

    async fn write_file<P: AsRef<Path> + Send>(
        &self,
        path: P,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let p = self.local(&path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, data).with_context(|| format!("fake write_file {}", p.display()))
    }

    async fn list_dir<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<Vec<String>> {
        let p = self.local(&path);
        let entries =
            std::fs::read_dir(&p).with_context(|| format!("fake list_dir {}", p.display()))?;
        let mut out = Vec::new();
        for e in entries {
            let e = e?;
            out.push(e.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        Ok(out)
    }

    async fn remove_file<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<()> {
        let p = self.local(&path);
        std::fs::remove_file(&p).with_context(|| format!("fake remove_file {}", p.display()))
    }

    async fn execute(&self, command: &str) -> anyhow::Result<String> {
        let cmds = self.commands.lock().unwrap();
        for (substr, output) in cmds.iter() {
            if command.contains(substr) {
                return Ok(output.clone());
            }
        }
        bail!("fake execute: no registered output for command `{command}`")
    }

    async fn file_exists<P: AsRef<Path> + Send>(&self, path: P) -> anyhow::Result<bool> {
        Ok(self.local(&path).exists())
    }
}
