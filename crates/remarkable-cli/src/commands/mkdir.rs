//! Implementation of `remarkable-cli mkdir <path> [--parents]`.
//!
//! Creates one or more folders by writing `<uuid>.metadata` files with
//! `type: CollectionType`. Collections have no `.content` file (the loader
//! enforces this in [`remarkable_metadata::metadata::DocumentEntry::from_raw`]).
//!
//! With `--parents`, missing intermediate folders are queued for creation
//! and an already-existing path is treated as a no-op success. Without it,
//! intermediate folders must already exist and the leaf must not.

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::cli::MkdirArgs;
use crate::commands::common::{self, CommandContext};
use remarkable_tablet::connection::TabletConnection;
use crate::error::CliError;
use remarkable_metadata::metadata::{ItemType, Parent, RawMetadata};
use crate::output::{self, OutputFormat};
use remarkable_tablet::tablet;
use remarkable_metadata::tree::{ChildLookup, DocumentTree};

#[derive(Serialize, Debug)]
pub struct MkdirOutput {
    pub path: String,
    /// Folders actually created. Empty when `--parents` is set and the
    /// full path already exists.
    pub created: Vec<CreatedFolder>,
}

#[derive(Serialize, Debug)]
pub struct CreatedFolder {
    pub uuid: Uuid,
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uuid: Option<Uuid>,
}

/// One folder queued for creation. UUIDs are minted up front so children
/// in the same `--parents` chain can reference their parent's UUID before
/// it lands on disk.
struct PendingFolder {
    uuid: Uuid,
    name: String,
    parent: Parent,
    path: String,
}

/// # Errors
/// Returns an error if the path is invalid, an intermediate component is
/// not a folder, an intermediate is missing without `--parents`, the leaf
/// already exists without `--parents`, or any remote write fails.
pub async fn execute(ctx: &CommandContext, args: &MkdirArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &MkdirArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args, ctx.no_restart()).await;
    session.ssh.disconnect().await;
    let out = result?;
    print_output(&out, ctx.format());
    Ok(())
}

/// Test-friendly core. Plans the folder writes against `tree`, then
/// brackets the writes with xochitl stop/start.
///
/// # Errors
/// See [`execute`].
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &MkdirArgs,
    no_restart: bool,
) -> anyhow::Result<MkdirOutput> {
    let pending = plan_creations(tree, &args.path, args.parents)?;

    if pending.is_empty() {
        return Ok(MkdirOutput {
            path: args.path.clone(),
            created: vec![],
        });
    }

    let now = Utc::now();
    let created = tablet::with_xochitl_stopped(conn, no_restart, || async {
        write_folders(conn, data_dir, &pending, now).await
    })
    .await?;

    Ok(MkdirOutput {
        path: args.path.clone(),
        created,
    })
}

fn plan_creations(
    tree: &DocumentTree,
    path: &str,
    parents: bool,
) -> anyhow::Result<Vec<PendingFolder>> {
    if !path.starts_with('/') {
        return Err(CliError::InvalidPath(format!("path must start with '/': {path}")).into());
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(CliError::InvalidPath("cannot mkdir root".to_string()).into());
    }
    if segments[0] == "trash" {
        return Err(CliError::InvalidPath(
            "cannot mkdir under /trash; trash is a virtual container".to_string(),
        )
        .into());
    }

    let mut current = Parent::Root;
    let mut current_path = String::new();
    let mut pending = Vec::new();

    for (i, segment) in segments.iter().enumerate() {
        current_path.push('/');
        current_path.push_str(segment);
        let is_last = i == segments.len() - 1;

        match tree.lookup_child(&current, segment) {
            ChildLookup::Entry(e) => {
                if !e.is_folder() {
                    return Err(CliError::InvalidPath(format!(
                        "'{segment}' is a {}, not a folder",
                        common::type_label(&e.kind)
                    ))
                    .into());
                }
                if is_last && !parents {
                    return Err(CliError::AlreadyExists(format!(
                        "folder already exists: {current_path}"
                    ))
                    .into());
                }
                current = Parent::Folder(e.uuid);
            }
            ChildLookup::Ambiguous => {
                return Err(CliError::InvalidPath(format!(
                    "ambiguous: multiple items named '{segment}' in the same folder \
                     — use a UUID instead"
                ))
                .into());
            }
            ChildLookup::Missing => {
                if !is_last && !parents {
                    return Err(CliError::NotFound(format!(
                        "intermediate folder '{segment}' missing in {current_path}; pass --parents"
                    ))
                    .into());
                }
                let uuid = Uuid::new_v4();
                pending.push(PendingFolder {
                    uuid,
                    name: (*segment).to_string(),
                    parent: current.clone(),
                    path: current_path.clone(),
                });
                current = Parent::Folder(uuid);
            }
        }
    }

    Ok(pending)
}

async fn write_folders<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    pending: &[PendingFolder],
    now: chrono::DateTime<Utc>,
) -> anyhow::Result<Vec<CreatedFolder>> {
    let mut out = Vec::with_capacity(pending.len());
    for p in pending {
        let metadata = RawMetadata {
            visible_name: p.name.clone(),
            item_type: ItemType::Collection,
            parent: p.parent.clone(),
            deleted: false,
            pinned: false,
            last_modified: now,
            metadata_modified: Some(now),
            version: 1,
            tags: vec![],
            last_opened: None,
        };
        let metadata_remote = format!("{data_dir}/{}.metadata", p.uuid);
        conn.write_file(&metadata_remote, &serde_json::to_vec(&metadata)?)
            .await?;
        out.push(CreatedFolder {
            uuid: p.uuid,
            name: p.name.clone(),
            path: p.path.clone(),
            parent_uuid: match p.parent {
                Parent::Folder(u) => Some(u),
                Parent::Root | Parent::Trash => None,
            },
        });
    }
    Ok(out)
}

fn print_output(out: &MkdirOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &MkdirOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &MkdirOutput) -> String {
    if o.created.is_empty() {
        return format!("{} already exists, nothing to create", o.path);
    }
    let mut lines = Vec::with_capacity(o.created.len() + 1);
    lines.push(format!("created {} folder(s)", o.created.len()));
    for c in &o.created {
        lines.push(format!("  - {}  uuid={}", c.path, c.uuid));
    }
    lines.join("\n")
}
