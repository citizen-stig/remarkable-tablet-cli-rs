//! Per-phase timing of SSH connection setup to the tablet.
//!
//! Usage: `cargo run --release -p remarkable-tablet --example connect_bench -- [host] [iterations]`
//! (defaults: `10.11.99.1`, 10). Auth uses ssh-agent only (`SSH_AUTH_SOCK`), like the CLI's first auth method.
//!
//! Phases mirror `SshConnection::connect` (src/connection/ssh.rs) but are timed separately; `e2e` runs the
//! real `SshConnection::connect` so changes there show up without editing this file.

use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::{self, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PublicKey};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncBufReadExt;

use remarkable_tablet::{ConnectOptions, SshConnection};

struct AcceptAll;

impl client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

const PHASES: [&str; 7] = [
    "tcp",
    "banner",
    "handshake",
    "auth",
    "sftp",
    "first_op",
    "e2e",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "10.11.99.1".to_string());
    let iterations: usize = match args.next() {
        Some(n) => n
            .parse()
            .map_err(|err| anyhow::anyhow!("iterations {n:?}: {err}"))?,
        None => 10,
    };
    let addr = format!("{host}:22");

    let mut samples: Vec<Vec<Duration>> = vec![Vec::new(); PHASES.len()];
    for _ in 0..iterations {
        for (phase, took) in run_once(&host, &addr).await?.into_iter().enumerate() {
            samples[phase].push(took);
        }
    }

    println!("{addr}, {iterations} iterations (ms)");
    println!("{:<10} {:>8} {:>8} {:>8}", "phase", "min", "median", "max");
    for (name, mut s) in PHASES.iter().zip(samples) {
        s.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!(
            "{name:<10} {:>8.1} {:>8.1} {:>8.1}",
            ms(s[0]),
            ms(s[s.len() / 2]),
            ms(s[s.len() - 1])
        );
    }
    Ok(())
}

/// One sample of every phase, in `PHASES` order.
async fn run_once(host: &str, addr: &str) -> anyhow::Result<Vec<Duration>> {
    // tcp + banner on a throwaway connection: time until the server's SSH id line arrives
    // isolates server accept/fork latency from crypto.
    let start = Instant::now();
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let tcp = start.elapsed();
    let start = Instant::now();
    let mut line = String::new();
    tokio::io::BufReader::new(stream)
        .read_line(&mut line)
        .await?;
    let banner = start.elapsed();
    anyhow::ensure!(
        line.starts_with("SSH-"),
        "expected SSH id line, got {line:?}"
    );

    // `connect_stream` returns once key exchange is done.
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let start = Instant::now();
    let mut handle =
        client::connect_stream(Arc::new(client::Config::default()), stream, AcceptAll).await?;
    let handshake = start.elapsed();

    let start = Instant::now();
    anyhow::ensure!(
        agent_auth(&mut handle).await?,
        "ssh-agent auth rejected by {addr}"
    );
    let auth = start.elapsed();

    let start = Instant::now();
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = SftpSession::new(channel.into_stream()).await?;
    let sftp_open = start.elapsed();

    let start = Instant::now();
    sftp.metadata("/").await?;
    let first_op = start.elapsed();
    handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await?;

    let opts = ConnectOptions {
        user: "root".to_string(),
        password: None,
        key_file: None,
        timeout: Duration::from_secs(5),
        verbose: false,
    };
    let start = Instant::now();
    let conn = SshConnection::connect(host, 22, &opts).await?;
    let e2e = start.elapsed();
    conn.disconnect().await;

    Ok(vec![tcp, banner, handshake, auth, sftp_open, first_op, e2e])
}

async fn agent_auth(handle: &mut Handle<AcceptAll>) -> anyhow::Result<bool> {
    let sock =
        std::env::var("SSH_AUTH_SOCK").map_err(|err| anyhow::anyhow!("SSH_AUTH_SOCK: {err}"))?;
    let mut agent = AgentClient::connect_uds(&sock).await?;
    for key in agent.request_identities().await? {
        let result = handle
            .authenticate_publickey_with("root", key, Some(HashAlg::Sha512), &mut agent)
            .await?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}
