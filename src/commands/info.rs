use serde::Serialize;
use uuid::Uuid;

use crate::cli::{GlobalOptions, InfoArgs};
use crate::commands::common::{self, ItemKind};
use crate::connection::TabletConnection;
use crate::error::{CliError, Result};
use crate::metadata::FileType;
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tree::DocumentTree;

#[derive(Serialize, Debug)]
pub struct InfoOutput {
    pub uuid: Uuid,
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ItemKind,
    pub file_type: Option<FileType>,
    pub deleted: bool,
    pub pinned: bool,
    pub tags: Vec<String>,
    /// Verbatim `.metadata` JSON.
    pub metadata: serde_json::Value,
    /// Verbatim `.content` JSON, or `null` for folders / when the file is missing.
    pub content: Option<serde_json::Value>,
}

pub async fn execute(global: &GlobalOptions, args: &InfoArgs) -> Result<()> {
    run(global, args).await.map_err(common::to_cli_error)
}

async fn run(global: &GlobalOptions, args: &InfoArgs) -> anyhow::Result<()> {
    let (ssh, cfg, tree) = common::connect_and_load_tree(global).await?;
    let result = run_with_conn(&ssh, &cfg.data_dir, &tree, args).await;
    ssh.disconnect().await;
    let info = result?;
    print_output(&info, cfg.format);
    Ok(())
}

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

    let path = common::entry_path(tree, entry);
    let metadata_path = format!("{data_dir}/{}.metadata", entry.uuid);
    let metadata_bytes = conn.read_file(&metadata_path).await?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata_bytes)?;

    // A single SFTP read; missing/unreadable `.content` is treated as `None`,
    // matching `tablet::load_one`. This drops a redundant `try_exists`
    // round-trip on the listing path.
    let content = if entry.is_document() {
        let content_path = format!("{data_dir}/{}.content", entry.uuid);
        match conn.read_file(&content_path).await {
            Ok(bytes) => Some(serde_json::from_slice::<serde_json::Value>(&bytes)?),
            Err(_) => None,
        }
    } else {
        None
    };

    Ok(InfoOutput {
        uuid: entry.uuid,
        path,
        name: entry.visible_name.clone(),
        kind: entry.item_type.into(),
        file_type: entry.file_type,
        deleted: entry.is_trashed(),
        pinned: entry.pinned,
        tags: entry.tags.clone(),
        metadata,
        content,
    })
}

fn print_output(info: &InfoOutput, format: OutputFormat) {
    match format {
        OutputFormat::Json => output::print_json(info),
        OutputFormat::Human => print_human(info),
    }
}

fn print_human(info: &InfoOutput) {
    println!("uuid:      {}", info.uuid);
    println!("path:      {}", info.path);
    println!("name:      {}", info.name);
    println!("type:      {}", common::type_label(info.kind, None));
    if let Some(ft) = info.file_type {
        println!("file_type: {}", common::file_type_label(ft));
    }
    if info.pinned {
        println!("pinned:    true");
    }
    if info.deleted {
        println!("deleted:   true");
    }
    if !info.tags.is_empty() {
        println!("tags:      {}", info.tags.join(", "));
    }
    println!();
    println!("metadata:");
    println!(
        "{}",
        serde_json::to_string_pretty(&info.metadata).unwrap_or_default()
    );
    println!();
    println!("content:");
    match &info.content {
        Some(c) => println!("{}", serde_json::to_string_pretty(c).unwrap_or_default()),
        None => println!("null"),
    }
}
