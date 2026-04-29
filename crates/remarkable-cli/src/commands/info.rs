use serde::Serialize;

use crate::cli::InfoArgs;
use crate::commands::common::{self, CommandContext, EntryView};
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use remarkable_metadata::path_resolver::{self, Resolved};
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::TabletConnection;

#[derive(Serialize, Debug)]
pub struct InfoOutput {
    #[serde(flatten)]
    pub entry: EntryView,
    /// Verbatim `.metadata` JSON.
    pub metadata: serde_json::Value,
    /// Verbatim `.content` JSON, or `null` when the file is missing/unreadable.
    pub content: Option<serde_json::Value>,
}

/// # Errors
/// Returns an error if connection fails, metadata cannot be loaded, or the path/UUID does not resolve.
pub async fn execute(ctx: &CommandContext, args: &InfoArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &InfoArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_conn(&session.ssh, ctx.data_dir(), &tree, args).await;
    session.ssh.disconnect().await;
    let info = result?;
    print_output(&info, ctx.format());
    Ok(())
}

/// # Errors
/// Returns an error if `args.path_or_uuid` does not resolve, or if the entry's
/// `.metadata` / `.content` files cannot be fetched or parsed.
pub async fn run_with_conn<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    tree: &DocumentTree,
    args: &InfoArgs,
) -> anyhow::Result<InfoOutput> {
    if args.path_or_uuid == "/" {
        return Err(CliError::InvalidPath(
            "info on root is not supported; use 'ls /' instead".to_string(),
        )
        .into());
    }

    let entry = match path_resolver::resolve(tree, &args.path_or_uuid)? {
        Resolved::Root => {
            return Err(CliError::InvalidPath(
                "info on root is not supported; use 'ls /' instead".to_string(),
            )
            .into());
        }
        Resolved::Entry(e) => e,
    };

    let metadata_path = format!("{data_dir}/{}.metadata", entry.uuid);
    let metadata_bytes = conn.read_file(&metadata_path).await?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata_bytes)?;

    // The tree only contains documents whose `.content` parsed cleanly at
    // load time, so a Document entry here is guaranteed loadable. Folders
    // and templates skip the read entirely.
    let content = if entry.is_document() {
        let content_path = format!("{data_dir}/{}.content", entry.uuid);
        let bytes = conn.read_file(&content_path).await?;
        Some(serde_json::from_slice::<serde_json::Value>(&bytes)?)
    } else {
        None
    };

    Ok(InfoOutput {
        entry: EntryView::from_entry(tree, entry),
        metadata,
        content,
    })
}

fn print_output(info: &InfoOutput, format: OutputFormat) {
    println!("{}", format_output(info, format));
}

/// # Panics
/// Panics if `info` cannot be serialized as JSON.
#[must_use]
pub fn format_output(info: &InfoOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(info),
        OutputFormat::Human => format_human(info),
    }
}

#[must_use]
pub fn format_human(info: &InfoOutput) -> String {
    let e = &info.entry;
    let mut lines = vec![
        format!("uuid:      {}", e.uuid),
        format!("path:      {}", e.path),
        format!("name:      {}", e.name),
        format!("type:      {}", common::type_label(&e.kind)),
    ];
    if let remarkable_metadata::metadata::ItemKind::Document { file_type, .. } = e.kind {
        lines.push(format!("file_type: {}", common::file_type_label(file_type)));
    }
    if e.pinned {
        lines.push("pinned:    true".to_string());
    }
    if e.deleted {
        lines.push("deleted:   true".to_string());
    }
    if !e.tags.is_empty() {
        lines.push(format!("tags:      {}", e.tags.join(", ")));
    }
    lines.push(String::new());
    lines.push("metadata:".to_string());
    lines.push(serde_json::to_string_pretty(&info.metadata).unwrap_or_default());
    lines.push(String::new());
    lines.push("content:".to_string());
    lines.push(match &info.content {
        Some(c) => serde_json::to_string_pretty(c).unwrap_or_default(),
        None => "null".to_string(),
    });
    lines.join("\n")
}
