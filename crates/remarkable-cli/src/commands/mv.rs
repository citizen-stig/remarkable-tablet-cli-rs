//! Implementation of `remarkable-cli mv <source> <dest-folder>`.
//!
//! Updates `parent` in the source's `.metadata`, plus timestamps and
//! `version`. Refuses moves that would form a cycle (folder into itself
//! or one of its own descendants), into a non-folder, or into trash —
//! `rm` is the way to trash an item.

use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::cli::MvArgs;
use crate::commands::common::{self, CommandContext, is_false};
use crate::commands::rename::bump_version;
use remarkable_tablet::connection::TabletConnection;
use crate::error::CliError;
use remarkable_metadata::metadata::{DocumentEntry, Parent};
use crate::output::{self, OutputFormat};
use remarkable_metadata::path_resolver::{self, Resolved};
use remarkable_tablet::tablet;
use remarkable_metadata::tree::{ChildLookup, DocumentTree, ListFilter};

#[derive(Serialize, Debug)]
pub struct MvOutput {
    pub uuid: Uuid,
    pub name: String,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "is_false", default)]
    pub no_op: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// # Errors
/// Returns an error if either path fails to resolve, the destination is
/// not a non-trashed folder, the move would form a cycle, or the remote
/// write fails.
pub async fn execute(ctx: &CommandContext, args: &MvArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &MvArgs) -> anyhow::Result<()> {
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
    args: &MvArgs,
    no_restart: bool,
) -> anyhow::Result<MvOutput> {
    let source = match path_resolver::resolve(tree, &args.source)? {
        Resolved::Root => {
            return Err(CliError::InvalidPath("cannot move root".to_string()).into());
        }
        Resolved::Entry(e) => e,
    };

    let (new_parent, dest_path) = resolve_destination(tree, &args.dest_folder)?;
    reject_cycles(tree, source, &new_parent)?;

    let from = tree
        .display_path(&source.uuid)
        .map_or_else(|| format!("/{}", source.visible_name), str::to_string);

    if source.parent == new_parent && !source.deleted {
        return Ok(MvOutput {
            uuid: source.uuid,
            name: source.visible_name.clone(),
            from,
            to: dest_path,
            no_op: true,
            warnings: vec![],
        });
    }

    path_resolver::ensure_not_reserved_trash_path(tree, &new_parent, &source.visible_name)?;

    let warnings = match tree.lookup_child(&new_parent, &source.visible_name) {
        ChildLookup::Missing => vec![],
        ChildLookup::Entry(_) | ChildLookup::Ambiguous => vec![format!(
            "destination already contains an entry named '{}'",
            source.visible_name
        )],
    };

    let metadata_remote = format!("{data_dir}/{}.metadata", source.uuid);
    let new_parent_str = parent_to_metadata_str(&new_parent);

    tablet::with_xochitl_stopped(conn, no_restart, || async {
        tablet::update_metadata(conn, &metadata_remote, |obj| {
            let now_ms = Utc::now().timestamp_millis();
            obj.insert("parent".into(), json!(new_parent_str));
            obj.insert("deleted".into(), json!(false));
            obj.insert("lastModified".into(), json!(now_ms));
            obj.insert("metadatamodified".into(), json!(now_ms));
            bump_version(obj);
        })
        .await
    })
    .await?;

    Ok(MvOutput {
        uuid: source.uuid,
        name: source.visible_name.clone(),
        from,
        to: dest_path,
        no_op: false,
        warnings,
    })
}

fn resolve_destination(tree: &DocumentTree, dest: &str) -> anyhow::Result<(Parent, String)> {
    match path_resolver::resolve(tree, dest)? {
        Resolved::Root => Ok((Parent::Root, "/".to_string())),
        Resolved::Entry(e) => {
            if !e.is_folder() {
                return Err(CliError::InvalidPath(format!(
                    "destination must be a folder, not a {}: {}",
                    common::type_label(&e.kind),
                    e.visible_name
                ))
                .into());
            }
            if e.is_trashed() {
                return Err(CliError::InvalidPath(format!(
                    "cannot move into trashed folder: {} (use rm to trash items)",
                    e.visible_name
                ))
                .into());
            }
            let path = tree
                .display_path(&e.uuid)
                .map_or_else(|| format!("/{}", e.visible_name), str::to_string);
            Ok((Parent::Folder(e.uuid), path))
        }
    }
}

fn reject_cycles(
    tree: &DocumentTree,
    source: &DocumentEntry,
    new_parent: &Parent,
) -> anyhow::Result<()> {
    let dest_uuid = match new_parent {
        Parent::Folder(u) => *u,
        Parent::Root | Parent::Trash => return Ok(()),
    };
    if dest_uuid == source.uuid {
        return Err(CliError::InvalidPath(format!(
            "cannot move '{}' into itself",
            source.visible_name
        ))
        .into());
    }
    if source.is_folder() {
        let descendants = tree.list_recursive(
            &Parent::Folder(source.uuid),
            None,
            ListFilter::all().include_trashed(),
        )?;
        if descendants.iter().any(|(_, e)| e.uuid == dest_uuid) {
            return Err(CliError::InvalidPath(format!(
                "cannot move folder '{}' into its own descendant",
                source.visible_name
            ))
            .into());
        }
    }
    Ok(())
}

/// Encode a [`Parent`] the same way `metadata::Parent::serialize` does.
/// Kept inline rather than reusing `serde_json::to_value(parent)` so
/// callers stay free of serializer plumbing for one-line writes.
fn parent_to_metadata_str(parent: &Parent) -> String {
    match parent {
        Parent::Root => String::new(),
        Parent::Trash => "trash".to_string(),
        Parent::Folder(u) => u.to_string(),
    }
}

fn print_output(out: &MvOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &MvOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &MvOutput) -> String {
    let mut lines = Vec::with_capacity(o.warnings.len() + 1);
    if o.no_op {
        lines.push(format!("no change: '{}' is already in '{}'", o.name, o.to));
    } else {
        lines.push(format!("moved: '{}' → '{}'  uuid={}", o.from, o.to, o.uuid));
    }
    for w in &o.warnings {
        lines.push(format!("warning: {w}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarkable_tablet::connection::FakeConnection;
    use remarkable_metadata::metadata::{FileType, ItemKind};
    use chrono::{TimeZone, Utc};

    fn make_folder(uuid: &str, name: &str, parent: Parent) -> DocumentEntry {
        let deleted = parent == Parent::Trash;
        DocumentEntry {
            uuid: Uuid::parse_str(uuid).unwrap(),
            visible_name: name.to_string(),
            kind: ItemKind::Folder,
            parent,
            deleted,
            pinned: false,
            last_modified: Utc.timestamp_millis_opt(1_710_000_000_000).unwrap(),
            version: 1,
            tags: vec![],
            last_opened: None,
        }
    }

    fn make_doc(uuid: &str, name: &str, parent: Parent, file_type: FileType) -> DocumentEntry {
        let deleted = parent == Parent::Trash;
        DocumentEntry {
            uuid: Uuid::parse_str(uuid).unwrap(),
            visible_name: name.to_string(),
            kind: ItemKind::Document {
                file_type,
                page_count: None,
            },
            parent,
            deleted,
            pinned: false,
            last_modified: Utc.timestamp_millis_opt(1_710_000_000_000).unwrap(),
            version: 1,
            tags: vec![],
            last_opened: None,
        }
    }

    #[test]
    fn parent_to_metadata_str_matches_serializer() {
        assert_eq!(parent_to_metadata_str(&Parent::Root), "");
        assert_eq!(parent_to_metadata_str(&Parent::Trash), "trash");
        let uuid = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        assert_eq!(
            parent_to_metadata_str(&Parent::Folder(uuid)),
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        );
    }

    #[tokio::test]
    async fn reject_moving_item_named_trash_to_root() {
        let folder = Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        let tree = DocumentTree::build(vec![
            make_folder(
                "aaaaaaaa-0000-0000-0000-000000000001",
                "Inbox",
                Parent::Root,
            ),
            make_doc(
                "bbbbbbbb-0000-0000-0000-000000000001",
                "trash",
                Parent::Folder(folder),
                FileType::Pdf,
            ),
        ]);
        let conn = FakeConnection::new();
        let args = MvArgs {
            source: "/Inbox/trash".to_string(),
            dest_folder: "/".to_string(),
        };

        let err = run_with_conn(&conn, "/xochitl", &tree, &args, false)
            .await
            .unwrap_err();
        let cli = common::to_cli_error(err);
        assert!(matches!(cli, CliError::InvalidPath(msg) if msg.contains("/trash")));
        assert!(conn.executed_commands().is_empty());
    }

    #[tokio::test]
    async fn reject_moving_into_real_root_trash_folder_by_uuid() {
        let tree = DocumentTree::build(vec![
            make_folder(
                "aaaaaaaa-0000-0000-0000-000000000001",
                "trash",
                Parent::Root,
            ),
            make_doc(
                "bbbbbbbb-0000-0000-0000-000000000001",
                "Notes",
                Parent::Root,
                FileType::Pdf,
            ),
        ]);
        let conn = FakeConnection::new();
        let args = MvArgs {
            source: "/Notes".to_string(),
            dest_folder: "aaaaaaaa-0000-0000-0000-000000000001".to_string(),
        };

        let err = run_with_conn(&conn, "/xochitl", &tree, &args, false)
            .await
            .unwrap_err();
        let cli = common::to_cli_error(err);
        assert!(matches!(cli, CliError::InvalidPath(msg) if msg.contains("/trash")));
        assert!(conn.executed_commands().is_empty());
    }
}
