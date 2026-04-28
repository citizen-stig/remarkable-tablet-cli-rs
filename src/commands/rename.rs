//! Implementation of `remarkable-cli rename <path-or-uuid> <new-name>`.
//!
//! Updates `visibleName` in the target's `.metadata` and bumps the
//! timestamps and `version` so xochitl picks up the change on next sync.
//! Source path/UUID resolution and duplicate-name detection mirror
//! [`crate::commands::upload`].

use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::cli::RenameArgs;
use crate::commands::common::{self, CommandContext, is_false};
use crate::connection::TabletConnection;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tablet;
use crate::tree::{ChildLookup, DocumentTree};

#[derive(Serialize, Debug)]
pub struct RenameOutput {
    pub uuid: Uuid,
    pub old_name: String,
    pub new_name: String,
    pub path: String,
    #[serde(skip_serializing_if = "is_false", default)]
    pub no_op: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// # Errors
/// Returns an error if the source does not resolve, the new name is
/// invalid, or the remote write fails.
pub async fn execute(ctx: &CommandContext, args: &RenameArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &RenameArgs) -> anyhow::Result<()> {
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
    args: &RenameArgs,
    no_restart: bool,
) -> anyhow::Result<RenameOutput> {
    validate_name(&args.new_name)?;

    let entry = match path_resolver::resolve(tree, &args.path_or_uuid)? {
        Resolved::Root => {
            return Err(CliError::InvalidPath("cannot rename root".to_string()).into());
        }
        Resolved::Entry(e) => e,
    };

    let old_name = entry.visible_name.clone();

    if args.new_name == old_name {
        let path = display_path(tree, entry, &args.new_name);
        return Ok(RenameOutput {
            uuid: entry.uuid,
            old_name,
            new_name: args.new_name.clone(),
            path,
            no_op: true,
            warnings: vec![],
        });
    }

    path_resolver::ensure_not_reserved_trash_path(tree, &entry.parent, &args.new_name)?;

    let warnings = match tree.lookup_child(&entry.parent, &args.new_name) {
        ChildLookup::Missing => vec![],
        ChildLookup::Entry(_) | ChildLookup::Ambiguous => vec![format!(
            "parent already contains an entry named '{}'",
            args.new_name
        )],
    };

    let metadata_remote = format!("{data_dir}/{}.metadata", entry.uuid);
    let new_name = args.new_name.clone();

    tablet::with_xochitl_stopped(conn, no_restart, || async {
        tablet::update_metadata(conn, &metadata_remote, |obj| {
            apply_rename(obj, &new_name, Utc::now().timestamp_millis());
        })
        .await
    })
    .await?;

    let path = parent_path(tree, entry).map_or_else(
        || format!("/{}", args.new_name),
        |p| format!("{p}/{}", args.new_name),
    );

    Ok(RenameOutput {
        uuid: entry.uuid,
        old_name,
        new_name: args.new_name.clone(),
        path,
        no_op: false,
        warnings,
    })
}

fn validate_name(name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        return Err(CliError::InvalidPath("name must not be empty".to_string()).into());
    }
    if name.contains('/') {
        return Err(CliError::InvalidPath(format!("name must not contain '/': {name:?}")).into());
    }
    if name.contains('\0') {
        return Err(CliError::InvalidPath("name must not contain NUL".to_string()).into());
    }
    Ok(())
}

fn apply_rename(obj: &mut serde_json::Map<String, serde_json::Value>, new_name: &str, now_ms: i64) {
    obj.insert("visibleName".into(), json!(new_name));
    obj.insert("lastModified".into(), json!(now_ms));
    obj.insert("metadatamodified".into(), json!(now_ms));
    bump_version(obj);
}

/// Increment the `version` field by 1 if present and an integer; otherwise
/// leave it alone. xochitl uses this for sync conflict detection — matching
/// its convention is safer than fixing a value or skipping the bump.
pub(crate) fn bump_version(obj: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(v) = obj.get("version").and_then(serde_json::Value::as_u64) {
        obj.insert("version".into(), json!(v + 1));
    }
}

/// Path of the entry's parent folder, or `None` if the entry sits at root
/// or its parent chain is broken. Used to compose the post-rename path.
fn parent_path<'a>(
    tree: &'a DocumentTree,
    entry: &crate::metadata::DocumentEntry,
) -> Option<&'a str> {
    match entry.parent {
        crate::metadata::Parent::Root => None,
        crate::metadata::Parent::Trash => Some("/trash"),
        crate::metadata::Parent::Folder(uuid) => tree.display_path(&uuid),
    }
}

/// Synthesize the target's full path under a hypothetical name. Used when
/// the rename is a no-op so we can still report a stable path.
fn display_path(tree: &DocumentTree, entry: &crate::metadata::DocumentEntry, name: &str) -> String {
    parent_path(tree, entry).map_or_else(|| format!("/{name}"), |p| format!("{p}/{name}"))
}

fn print_output(out: &RenameOutput, format: OutputFormat) {
    println!("{}", format_output(out, format));
}

#[must_use]
pub fn format_output(out: &RenameOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(out),
        OutputFormat::Human => format_human(out),
    }
}

fn format_human(o: &RenameOutput) -> String {
    let mut lines = Vec::with_capacity(o.warnings.len() + 1);
    if o.no_op {
        lines.push(format!(
            "no change: '{}' already named '{}'",
            o.path, o.new_name
        ));
    } else {
        lines.push(format!(
            "renamed: '{}' → '{}'  uuid={}",
            o.old_name, o.new_name, o.uuid
        ));
    }
    for w in &o.warnings {
        lines.push(format!("warning: {w}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::FakeConnection;
    use crate::metadata::{DocumentEntry, FileType, ItemKind, Parent};
    use chrono::{TimeZone, Utc};

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

    #[tokio::test]
    async fn reject_renaming_root_item_to_trash() {
        let tree = DocumentTree::build(vec![make_doc(
            "11111111-1111-1111-1111-111111111111",
            "Notes",
            Parent::Root,
            FileType::Pdf,
        )]);
        let conn = FakeConnection::new();
        let args = RenameArgs {
            path_or_uuid: "/Notes".to_string(),
            new_name: "trash".to_string(),
        };

        let err = run_with_conn(&conn, "/xochitl", &tree, &args, false)
            .await
            .unwrap_err();
        let cli = err.downcast_ref::<CliError>().unwrap();
        assert!(matches!(cli, CliError::InvalidPath(msg) if msg.contains("/trash")));
        assert!(conn.executed_commands().is_empty());
    }
}
