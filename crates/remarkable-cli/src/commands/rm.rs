//! Implementation of `remarkable-cli rm <paths>... [--permanent] [--recursive]`.
//!
//! Two modes:
//! - **Soft delete** (default): set `parent: "trash"` and `deleted: true`
//!   on each target's `.metadata`. Children stay parented to the trashed
//!   ancestor; they're reachable as `/trash/<folder>/<child>` and a later
//!   `mv <folder> /Some/Place` un-trashes the whole subtree atomically
//!   (see `mv`'s `deleted: false` write).
//! - **Permanent delete** (`--permanent`): remove every file whose name
//!   matches `<uuid>` or `<uuid>.*` under the data dir, plus matching
//!   directories. Metadata is removed last so a partial failure leaves
//!   an item visible-but-broken instead of orphaning its source files.
//!
//! `--recursive` is required for non-empty folders in both modes (safety
//! gate). For permanent delete it also expands the UUID set to include
//! every descendant.

use std::collections::HashSet;

use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::cli::RmArgs;
use crate::commands::common::{self, CommandContext, is_false};
use crate::commands::rename::bump_version;
use remarkable_tablet::connection::{RemoteEntry, RemoteFileKind, TabletConnection};
use crate::error::CliError;
use remarkable_metadata::metadata::{DocumentEntry, ItemKind, Parent};
use crate::output::{self, OutputFormat};
use remarkable_metadata::path_resolver::{self, Resolved};
use remarkable_tablet::tablet;
use remarkable_metadata::tree::{DocumentTree, ListFilter};

#[derive(Serialize, Debug)]
pub struct RmOutput {
    pub permanent: bool,
    pub deleted: Vec<DeletedItem>,
    /// Number of UUIDs whose files were removed under `--permanent
    /// --recursive`. `None` when the count equals `deleted.len()` (i.e.,
    /// soft delete or non-recursive permanent delete).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descendant_uuids_removed: Option<usize>,
    #[serde(skip_serializing_if = "is_false", default)]
    pub no_op: bool,
}

#[derive(Serialize, Debug)]
pub struct DeletedItem {
    pub uuid: Uuid,
    pub name: String,
    pub path: String,
    #[serde(flatten)]
    pub kind: ItemKind,
}

impl DeletedItem {
    fn from_entry(tree: &DocumentTree, entry: &DocumentEntry) -> Self {
        let path = tree
            .display_path(&entry.uuid)
            .map(str::to_string)
            .unwrap_or_else(|| format!("/{}", entry.visible_name));
        Self {
            uuid: entry.uuid,
            name: entry.visible_name.clone(),
            path,
            kind: entry.kind.clone(),
        }
    }
}

/// # Errors
/// Returns an error if any path fails to resolve, a non-empty folder is
/// targeted without `--recursive`, or any remote write fails.
pub async fn execute(ctx: &CommandContext, args: &RmArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &RmArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args, ctx.no_restart()).await;
    session.ssh.disconnect().await;
    let out = result?;
    print_output(&out, ctx.format());
    Ok(())
}

/// Test-friendly core. See [`execute`].
///
/// # Errors
/// See [`execute`].
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &RmArgs,
    no_restart: bool,
) -> anyhow::Result<RmOutput> {
    if args.paths.is_empty() {
        return Err(CliError::InvalidPath("no paths given".to_string()).into());
    }

    let targets = resolve_targets(tree, &args.paths)?;
    enforce_recursive_gate(tree, &targets, args.recursive)?;

    let deleted: Vec<DeletedItem> = targets
        .iter()
        .map(|t| DeletedItem::from_entry(tree, t))
        .collect();

    if args.permanent {
        let uuids = expand_for_permanent(tree, &targets, args.recursive)?;
        let descendant_uuids_removed = if uuids.len() > targets.len() {
            Some(uuids.len() - targets.len())
        } else {
            None
        };

        tablet::with_xochitl_stopped(conn, no_restart, || async {
            permanent_delete_all(conn, data_dir, &uuids).await
        })
        .await?;

        Ok(RmOutput {
            permanent: true,
            deleted,
            descendant_uuids_removed,
            no_op: false,
        })
    } else {
        let pending: Vec<&DocumentEntry> = targets
            .iter()
            .copied()
            .filter(|e| !is_already_trashed(e))
            .collect();

        if pending.is_empty() {
            return Ok(RmOutput {
                permanent: false,
                deleted,
                descendant_uuids_removed: None,
                no_op: true,
            });
        }

        tablet::with_xochitl_stopped(conn, no_restart, || async {
            soft_delete_all(conn, data_dir, &pending).await
        })
        .await?;

        Ok(RmOutput {
            permanent: false,
            deleted,
            descendant_uuids_removed: None,
            no_op: false,
        })
    }
}

fn resolve_targets<'a>(
    tree: &'a DocumentTree,
    paths: &[String],
) -> anyhow::Result<Vec<&'a DocumentEntry>> {
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for input in paths {
        match path_resolver::resolve(tree, input)? {
            Resolved::Root => {
                return Err(CliError::InvalidPath("cannot rm root".to_string()).into());
            }
            Resolved::Entry(e) => {
                if seen.insert(e.uuid) {
                    out.push(e);
                }
            }
        }
    }
    Ok(out)
}

fn enforce_recursive_gate(
    tree: &DocumentTree,
    targets: &[&DocumentEntry],
    recursive: bool,
) -> anyhow::Result<()> {
    if recursive {
        return Ok(());
    }
    for t in targets {
        if t.is_folder() && tree.children_count(&Parent::Folder(t.uuid)) > 0 {
            return Err(CliError::InvalidPath(format!(
                "folder '{}' is not empty; pass --recursive to delete it",
                t.visible_name
            ))
            .into());
        }
    }
    Ok(())
}

/// Build the full UUID set whose files will be wiped under `--permanent`.
/// Targets always included; descendants of folder targets included only
/// when `--recursive` was given (the gate already rejected non-empty
/// folders without `-r`).
fn expand_for_permanent(
    tree: &DocumentTree,
    targets: &[&DocumentEntry],
    recursive: bool,
) -> anyhow::Result<Vec<Uuid>> {
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        if seen.insert(t.uuid) {
            out.push(t.uuid);
        }
        if recursive && t.is_folder() {
            let descendants = tree.list_recursive(
                &Parent::Folder(t.uuid),
                None,
                ListFilter::all().include_trashed(),
            )?;
            for (_, e) in descendants {
                if seen.insert(e.uuid) {
                    out.push(e.uuid);
                }
            }
        }
    }
    Ok(out)
}

fn is_already_trashed(e: &DocumentEntry) -> bool {
    e.parent == Parent::Trash && e.deleted
}

async fn soft_delete_all<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    targets: &[&DocumentEntry],
) -> anyhow::Result<()> {
    for t in targets {
        let path = format!("{data_dir}/{}.metadata", t.uuid);
        tablet::update_metadata(conn, &path, |obj| {
            let now_ms = Utc::now().timestamp_millis();
            obj.insert("parent".into(), json!("trash"));
            obj.insert("deleted".into(), json!(true));
            obj.insert("lastModified".into(), json!(now_ms));
            obj.insert("metadatamodified".into(), json!(now_ms));
            bump_version(obj);
        })
        .await?;
    }
    Ok(())
}

async fn permanent_delete_all<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    uuids: &[Uuid],
) -> anyhow::Result<()> {
    let entries = conn.read_dir(data_dir).await?;
    for uuid in uuids {
        wipe_uuid(conn, data_dir, *uuid, &entries).await?;
    }
    Ok(())
}

/// Remove every top-level entry whose name is exactly `<uuid>` (the page
/// directory) or starts with `<uuid>.` (metadata, content, source, page-
/// data, thumbnails dir, etc.). Auxiliary entries are removed first so
/// the metadata file falls last — that way a mid-batch failure leaves
/// xochitl seeing the (now broken) item, which the user can retry,
/// rather than orphaning hundreds of MB of source files.
async fn wipe_uuid<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    uuid: Uuid,
    entries: &[RemoteEntry],
) -> anyhow::Result<()> {
    let uuid_str = uuid.to_string();
    let metadata_name = format!("{uuid_str}.metadata");
    let prefix = format!("{uuid_str}.");

    let matching: Vec<&RemoteEntry> = entries
        .iter()
        .filter(|e| e.name == uuid_str || e.name.starts_with(&prefix))
        .collect();

    for e in &matching {
        if e.name == metadata_name {
            continue;
        }
        let path = format!("{data_dir}/{}", e.name);
        match e.metadata.kind {
            RemoteFileKind::Dir => conn.remove_dir_all(&path).await?,
            RemoteFileKind::File | RemoteFileKind::Other => conn.remove_file(&path).await?,
        }
    }

    if matching.iter().any(|e| e.name == metadata_name) {
        conn.remove_file(&format!("{data_dir}/{metadata_name}"))
            .await?;
    }

    Ok(())
}

fn print_output(out: &RmOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &RmOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &RmOutput) -> String {
    if o.no_op {
        return "no change: all targets were already in trash".to_string();
    }
    let mode = if o.permanent {
        "permanently deleted"
    } else {
        "trashed"
    };
    let mut lines = Vec::with_capacity(o.deleted.len() + 2);
    lines.push(format!("{} {} item(s)", mode, o.deleted.len()));
    for d in &o.deleted {
        lines.push(format!(
            "  - {}  {}  uuid={}",
            d.path,
            common::type_label(&d.kind),
            d.uuid
        ));
    }
    if let Some(n) = o.descendant_uuids_removed {
        lines.push(format!("  + {n} descendant UUIDs wiped"));
    }
    lines.join("\n")
}
