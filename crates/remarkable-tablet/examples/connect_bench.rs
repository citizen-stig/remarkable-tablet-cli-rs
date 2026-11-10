//! Timing of a cold `ls`: SSH connect + metadata load + disconnect, as every CLI command pays it.
//!
//! Usage: `cargo run --release -p remarkable-tablet --example connect_bench -- [host] [iterations] [expected_hash] [--phases]`
//! (defaults: `10.11.99.1`, 10). Auth uses ssh-agent only (`SSH_AUTH_SOCK`), like the CLI's first auth method.
//!
//! Each iteration runs the real `SshConnection::connect` and `load_all_metadata_full`, so changes there show up
//! without editing this file. One untimed warm-up iteration runs first. The loaded entries are hashed; if
//! `expected_hash` is given, any other hash is an error, so a change that breaks loading fails the run.
//!
//! `--phases` additionally times each connect phase separately on throwaway connections, for profiling.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::{self, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncBufReadExt;

use remarkable_tablet::{ConnectOptions, SshConnection, load_all_metadata_full};

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

struct AcceptAll;

impl client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

const COLD_PHASES: [&str; 4] = ["connect", "metadata", "disconnect", "cold_ls"];
const CONNECT_PHASES: [&str; 6] = ["tcp", "banner", "handshake", "auth", "sftp", "first_op"];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (flags, positional): (Vec<String>, Vec<String>) =
        std::env::args().skip(1).partition(|a| a.starts_with("--"));
    let phases = match flags.as_slice() {
        [] => false,
        [f] if f == "--phases" => true,
        other => anyhow::bail!("unknown flags {other:?}, expected only --phases"),
    };
    let mut args = positional.into_iter();
    let host = args.next().unwrap_or_else(|| "10.11.99.1".to_string());
    let iterations: usize = match args.next() {
        Some(n) => n
            .parse()
            .map_err(|err| anyhow::anyhow!("iterations {n:?}: {err}"))?,
        None => 10,
    };
    let expected_hash = args.next();
    let addr = format!("{host}:22");

    let (_, hash, items) = cold_ls(&host).await?;
    println!("{addr}, {iterations} iterations + 1 warm-up (ms), {items} items, hash {hash}");
    if let Some(expected) = &expected_hash {
        anyhow::ensure!(
            *expected == hash,
            "expected hash {expected}, got {hash} ({items} items)"
        );
    }

    let mut cold: Vec<Vec<Duration>> = vec![Vec::new(); COLD_PHASES.len()];
    for _ in 0..iterations {
        let (times, got, _) = cold_ls(&host).await?;
        anyhow::ensure!(
            got == hash,
            "hash changed mid-run: first {hash}, then {got}"
        );
        for (phase, took) in times.into_iter().enumerate() {
            cold[phase].push(took);
        }
    }
    let medians = print_table(&COLD_PHASES, cold);

    if phases {
        let mut samples: Vec<Vec<Duration>> = vec![Vec::new(); CONNECT_PHASES.len()];
        for _ in 0..iterations {
            for (phase, took) in connect_phases(&addr).await?.into_iter().enumerate() {
                samples[phase].push(took);
            }
        }
        print_table(&CONNECT_PHASES, samples);
    }

    println!("cold_ls_ms: {:.1}", medians[3]);
    Ok(())
}

/// Prints min/median/max per phase and returns the medians in ms.
fn print_table(names: &[&str], samples: Vec<Vec<Duration>>) -> Vec<f64> {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!("{:<10} {:>8} {:>8} {:>8}", "phase", "min", "median", "max");
    names
        .iter()
        .zip(samples)
        .map(|(name, mut s)| {
            s.sort();
            let median = ms(s[s.len() / 2]);
            println!(
                "{name:<10} {:>8.1} {median:>8.1} {:>8.1}",
                ms(s[0]),
                ms(s[s.len() - 1])
            );
            median
        })
        .collect()
}

/// One cold `ls`: `[connect, metadata, disconnect, total]`, the hash of the loaded entries, and their count.
async fn cold_ls(host: &str) -> anyhow::Result<(Vec<Duration>, String, usize)> {
    let opts = ConnectOptions {
        user: "root".to_string(),
        password: None,
        key_file: None,
        timeout: Duration::from_secs(5),
        verbose: false,
    };
    let start = Instant::now();
    let conn = SshConnection::connect(host, 22, &opts).await?;
    let connect = start.elapsed();

    let phase = Instant::now();
    let (mut entries, diag) = load_all_metadata_full(&conn, DATA_DIR).await?;
    let metadata = phase.elapsed();

    let phase = Instant::now();
    conn.disconnect().await;
    let disconnect = phase.elapsed();
    let total = start.elapsed();

    entries.sort_by_key(|e| e.uuid);
    let mut hasher = DefaultHasher::new();
    serde_json::to_vec(&entries)?.hash(&mut hasher);
    diag.parse_failures.hash(&mut hasher);
    for (uuid, err) in &diag.content_failures {
        (uuid.as_bytes(), err).hash(&mut hasher);
    }
    let hash = format!("{:016x}", hasher.finish());

    Ok((
        vec![connect, metadata, disconnect, total],
        hash,
        entries.len(),
    ))
}

/// One sample of every connect phase, in `CONNECT_PHASES` order, on throwaway connections.
async fn connect_phases(addr: &str) -> anyhow::Result<Vec<Duration>> {
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

    Ok(vec![tcp, banner, handshake, auth, sftp_open, first_op])
}

async fn agent_auth(handle: &mut Handle<AcceptAll>) -> anyhow::Result<bool> {
    let sock =
        std::env::var("SSH_AUTH_SOCK").map_err(|err| anyhow::anyhow!("SSH_AUTH_SOCK: {err}"))?;
    let mut agent = AgentClient::connect_uds(&sock).await?;
    for key in agent.request_identities().await? {
        let result = handle
            .authenticate_publickey_with(
                "root",
                key.public_key().into_owned(),
                Some(HashAlg::Sha512),
                &mut agent,
            )
            .await?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}
