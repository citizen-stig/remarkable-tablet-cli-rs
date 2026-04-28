//! Implementation of `remarkable-cli backup <local_dir>`.
//!
//! Pulls the entire xochitl tree under `<local_dir>/xochitl/`, plus
//! `/etc/version` saved as `<local_dir>/version`, and emits a
//! `backup_manifest.json` describing what was copied.
//!
//! Modes:
//! - default: copy every regular file under the remote xochitl dir.
//! - `--incremental`: skip files whose local copy has an mtime
//!   greater-than-or-equal-to the remote mtime (rsync `-u` semantics,
//!   minus the size check — mtime alone is sufficient because the
//!   tablet stamps files when xochitl writes them).
//! - `--dry-run`: walk + filter, then report the plan without writing.
//!
//! Backup is *not* a sync: local files that no longer exist remotely are
//! left in place. Surprising behaviour to remove without an explicit
//! deletion gesture.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context;
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::cli::BackupArgs;
use crate::commands::common::{self, CommandContext};
use crate::connection::TabletConnection;
use crate::error::CliError;
use crate::output::OutputFormat;
use crate::transfer::{self, TRANSFER_CONCURRENCY, WalkedFile};

const MANIFEST_FILENAME: &str = "backup_manifest.json";
const XOCHITL_SUBDIR: &str = "xochitl";
const FIRMWARE_FILENAME: &str = "version";
const FIRMWARE_REMOTE_PATH: &str = "/etc/version";

/// Wire format for the JSON envelope `backup` emits to stdout.
#[derive(Serialize, Debug)]
pub struct BackupOutput {
    pub backup_path: PathBuf,
    pub timestamp: DateTime<Utc>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub copied: usize,
    pub skipped: usize,
    pub incremental: bool,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
}

/// Persisted on-disk manifest at `<backup_path>/backup_manifest.json`.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct BackupManifest {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<ManifestEntry>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Path relative to `<backup_path>/xochitl/`.
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<DateTime<Utc>>,
}

const MANIFEST_VERSION: u32 = 1;

/// # Errors
/// Returns an error if the SSH connection fails, the remote walk fails,
/// or any local file write fails.
pub async fn execute(ctx: &CommandContext, args: &BackupArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &BackupArgs) -> anyhow::Result<()> {
    let session = ctx.connect().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), args).await;
    session.ssh.disconnect().await;
    let output = result?;
    print_output(&output, ctx.format());
    Ok(())
}

/// Test-friendly core: takes any `TabletConnection`, performs the walk
/// plus copy plus manifest write, returns the [`BackupOutput`].
///
/// # Errors
/// Returns an error if walking the remote tree, reading any remote
/// file, writing any local file, or serializing the manifest fails.
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    args: &BackupArgs,
) -> anyhow::Result<BackupOutput> {
    let timestamp = Utc::now();
    let local_dir = &args.local_dir;
    let xochitl_local = local_dir.join(XOCHITL_SUBDIR);

    let mut files = transfer::walk_remote(conn, data_dir)
        .await
        .with_context(|| format!("walk {data_dir}"))?;
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    let total_files = files.len();
    let total_bytes: u64 = files.iter().map(|f| f.size.unwrap_or(0)).sum();
    let manifest_entries: Vec<ManifestEntry> =
        files.iter().map(walked_to_manifest_entry).collect();

    let plan: Vec<WalkedFile> = if args.incremental {
        filter_incremental(files, &xochitl_local).await
    } else {
        files
    };

    let copied = plan.len();
    let skipped = total_files.saturating_sub(copied);

    if args.dry_run {
        return Ok(BackupOutput {
            backup_path: local_dir.clone(),
            timestamp,
            file_count: total_files,
            total_bytes,
            copied,
            skipped,
            incremental: args.incremental,
            dry_run: true,
            manifest_path: None,
            firmware_version: None,
        });
    }

    tokio::fs::create_dir_all(&xochitl_local)
        .await
        .with_context(|| format!("create {}", xochitl_local.display()))?;

    let jobs: Vec<(String, PathBuf)> = plan
        .iter()
        .map(|f| (f.remote_path.clone(), xochitl_local.join(&f.rel_path)))
        .collect();
    transfer::download_many(conn, jobs)
        .await
        .context("download xochitl tree")?;

    let firmware_version = match conn.read_file(FIRMWARE_REMOTE_PATH).await {
        Ok(bytes) => {
            let firmware_local = local_dir.join(FIRMWARE_FILENAME);
            tokio::fs::write(&firmware_local, &bytes)
                .await
                .with_context(|| format!("write {}", firmware_local.display()))?;
            Some(String::from_utf8_lossy(&bytes).trim().to_string())
        }
        Err(_) => None,
    };

    let manifest = BackupManifest {
        version: MANIFEST_VERSION,
        timestamp,
        firmware_version: firmware_version.clone(),
        file_count: manifest_entries.len(),
        total_bytes,
        files: manifest_entries,
    };
    let manifest_path = local_dir.join(MANIFEST_FILENAME);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
    tokio::fs::write(&manifest_path, &manifest_bytes)
        .await
        .with_context(|| format!("write {}", manifest_path.display()))?;

    Ok(BackupOutput {
        backup_path: local_dir.clone(),
        timestamp,
        file_count: total_files,
        total_bytes,
        copied,
        skipped,
        incremental: args.incremental,
        dry_run: false,
        manifest_path: Some(manifest_path),
        firmware_version,
    })
}

async fn filter_incremental(files: Vec<WalkedFile>, xochitl_local: &Path) -> Vec<WalkedFile> {
    let mut kept: Vec<WalkedFile> = stream::iter(files)
        .map(|file| async move {
            let local_path = xochitl_local.join(&file.rel_path);
            if should_copy(&local_path, file.mtime).await {
                Some(file)
            } else {
                None
            }
        })
        .buffer_unordered(TRANSFER_CONCURRENCY)
        .filter_map(|x| async move { x })
        .collect()
        .await;
    kept.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    kept
}

async fn should_copy(local_path: &Path, remote_mtime: Option<SystemTime>) -> bool {
    let Ok(local_meta) = tokio::fs::metadata(local_path).await else {
        return true; // missing locally -> must copy
    };
    let Ok(local_mtime) = local_meta.modified() else {
        return true;
    };
    match remote_mtime {
        Some(remote) => remote > local_mtime,
        None => true, // no remote mtime -> assume changed
    }
}

fn walked_to_manifest_entry(f: &WalkedFile) -> ManifestEntry {
    ManifestEntry {
        path: f.rel_path.to_string_lossy().into_owned(),
        size: f.size.unwrap_or(0),
        mtime: f.mtime.and_then(systemtime_to_datetime),
    }
}

fn systemtime_to_datetime(t: SystemTime) -> Option<DateTime<Utc>> {
    let dur = t.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::<Utc>::from_timestamp(i64::try_from(dur.as_secs()).ok()?, dur.subsec_nanos())
}

fn print_output(output: &BackupOutput, format: OutputFormat) {
    println!("{}", format_output(output, format));
}

#[must_use]
pub fn format_output(output: &BackupOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => crate::output::render_json(output),
        OutputFormat::Human => format_human(output),
    }
}

#[must_use]
fn format_human(o: &BackupOutput) -> String {
    let mut lines = vec![
        format!("backup_path:      {}", o.backup_path.display()),
        format!("timestamp:        {}", o.timestamp.to_rfc3339()),
        format!("file_count:       {}", o.file_count),
        format!("total_bytes:      {}", o.total_bytes),
        format!("copied:           {}", o.copied),
        format!("skipped:          {}", o.skipped),
        format!("incremental:      {}", o.incremental),
        format!("dry_run:          {}", o.dry_run),
    ];
    if let Some(fw) = &o.firmware_version {
        lines.push(format!("firmware_version: {fw}"));
    }
    if let Some(path) = &o.manifest_path {
        lines.push(format!("manifest_path:    {}", path.display()));
    }
    lines.join("\n")
}
