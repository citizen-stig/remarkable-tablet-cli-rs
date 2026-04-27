use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::{GlobalOptions, LsArgs};
use crate::commands::common;
use crate::error::{CliError, Result};
use crate::metadata::{DocumentEntry, FileType, ItemType, Parent};
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tree::{self, DocumentTree};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Folder,
    Document,
    Template,
}

impl From<ItemType> for ItemKind {
    fn from(t: ItemType) -> Self {
        match t {
            ItemType::Collection => ItemKind::Folder,
            ItemType::Document => ItemKind::Document,
            ItemType::Template => ItemKind::Template,
        }
    }
}

#[derive(Serialize, Debug)]
pub struct LsItem {
    pub uuid: Uuid,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ItemKind,
    pub file_type: Option<FileType>,
    pub parent_uuid: Option<Uuid>,
    pub path: String,
    pub modified: DateTime<Utc>,
    pub last_opened: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub deleted: bool,
    pub children_count: Option<usize>,
    pub page_count: Option<u32>,
    /// 0 for direct children of the listed folder; only set for recursive listings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct TreeNode {
    /// `None` only for the synthetic root node (`/`).
    pub uuid: Option<Uuid>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ItemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_type: Option<FileType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_opened: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub pinned: bool,
    pub children: Vec<TreeNode>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug)]
pub enum LsOutput {
    Flat(Vec<LsItem>),
    Tree(TreeNode),
}

pub async fn execute(global: &GlobalOptions, args: &LsArgs) -> Result<()> {
    run(global, args).await.map_err(common::to_cli_error)
}

async fn run(global: &GlobalOptions, args: &LsArgs) -> anyhow::Result<()> {
    let (ssh, cfg, tree) = common::connect_and_load_tree(global).await?;
    let result = run_with_tree(&tree, args);
    ssh.disconnect().await;
    let output = result?;
    print_output(&output, cfg.format);
    Ok(())
}

pub fn run_with_tree(tree: &DocumentTree, args: &LsArgs) -> anyhow::Result<LsOutput> {
    let target = resolve_target(tree, args)?;

    if args.tree {
        let root = build_tree_node(tree, &target, args);
        Ok(LsOutput::Tree(root))
    } else if args.recursive || args.depth.is_some() {
        Ok(LsOutput::Flat(build_recursive(tree, &target, args)?))
    } else {
        Ok(LsOutput::Flat(build_flat(tree, &target, args)))
    }
}

struct Target {
    parent: Parent,
    /// Display name for tree mode's root node (e.g., `"/"`, `"Work"`).
    name: String,
}

fn resolve_target(tree: &DocumentTree, args: &LsArgs) -> anyhow::Result<Target> {
    let raw = args.path_or_uuid.as_deref().unwrap_or("/");
    match path_resolver::resolve(tree, raw)? {
        Resolved::Root => Ok(Target {
            parent: Parent::Root,
            name: "/".to_string(),
        }),
        Resolved::Entry(e) if e.is_folder() => Ok(Target {
            parent: Parent::Folder(e.uuid),
            name: e.visible_name.clone(),
        }),
        Resolved::Entry(e) => {
            Err(CliError::InvalidPath(format!("'{}' is not a folder", e.visible_name)).into())
        }
    }
}

// ---------------------------------------------------------------------------
// Flat listing (direct children)
// ---------------------------------------------------------------------------

fn build_flat(tree: &DocumentTree, target: &Target, args: &LsArgs) -> Vec<LsItem> {
    let entries = tree.list_children(
        &target.parent,
        args.include_trashed,
        args.documents_only,
        args.folders_only,
        args.sort.as_ref(),
    );
    entries
        .into_iter()
        .map(|e| to_ls_item(tree, e, None))
        .collect()
}

// ---------------------------------------------------------------------------
// Flat recursive listing
// ---------------------------------------------------------------------------

fn build_recursive(
    tree: &DocumentTree,
    target: &Target,
    args: &LsArgs,
) -> anyhow::Result<Vec<LsItem>> {
    let mut pairs = tree.list_recursive(
        &target.parent,
        args.depth,
        args.include_trashed,
        args.documents_only,
        args.folders_only,
        args.sort.as_ref(),
    )?;
    // Trashed items live under `Parent::Trash`, which is not a descendant of
    // `Parent::Root`. Walk the trash subtree explicitly when the user asked
    // for it from a root listing.
    if args.include_trashed && matches!(target.parent, Parent::Root) {
        pairs.extend(tree.list_recursive(
            &Parent::Trash,
            args.depth,
            true,
            args.documents_only,
            args.folders_only,
            args.sort.as_ref(),
        )?);
    }
    Ok(pairs
        .into_iter()
        .map(|(depth, e)| to_ls_item(tree, e, Some(depth)))
        .collect())
}

fn to_ls_item(tree: &DocumentTree, e: &DocumentEntry, depth: Option<u32>) -> LsItem {
    let parent_uuid = match &e.parent {
        Parent::Folder(u) => Some(*u),
        Parent::Root | Parent::Trash => None,
    };
    let path = path_resolver::resolve_uuid_to_path(tree, &e.uuid)
        .unwrap_or_else(|_| format!("/{}", e.visible_name));
    let children_count = if e.is_folder() {
        Some(tree.children_count(&Parent::Folder(e.uuid)))
    } else {
        None
    };
    LsItem {
        uuid: e.uuid,
        name: e.visible_name.clone(),
        kind: e.item_type.into(),
        file_type: e.file_type,
        parent_uuid,
        path,
        modified: e.last_modified,
        last_opened: e.last_opened,
        tags: e.tags.clone(),
        pinned: e.pinned,
        deleted: e.is_trashed(),
        children_count,
        page_count: e.page_count,
        depth,
    }
}

// ---------------------------------------------------------------------------
// Tree listing
// ---------------------------------------------------------------------------

fn build_tree_node(tree: &DocumentTree, target: &Target, args: &LsArgs) -> TreeNode {
    let (uuid, kind, file_type, last_opened, tags, pinned) = match &target.parent {
        Parent::Folder(u) => {
            let entry = tree.get(u).expect("target folder exists in tree");
            (
                Some(entry.uuid),
                ItemKind::Folder,
                entry.file_type,
                entry.last_opened,
                entry.tags.clone(),
                entry.pinned,
            )
        }
        _ => (None, ItemKind::Folder, None, None, Vec::new(), false),
    };

    let mut children = build_tree_children(tree, &target.parent, args, 0);
    // From a root listing with --include-trashed, splice the Trash subtree
    // in as well so the returned tree is self-contained.
    if args.include_trashed && matches!(target.parent, Parent::Root) {
        children.extend(build_tree_children(tree, &Parent::Trash, args, 0));
    }

    TreeNode {
        uuid,
        name: target.name.clone(),
        kind,
        file_type,
        last_opened,
        tags,
        pinned,
        children,
    }
}

fn build_tree_children(
    tree: &DocumentTree,
    parent: &Parent,
    args: &LsArgs,
    current_depth: u32,
) -> Vec<TreeNode> {
    if let Some(max) = args.depth
        && current_depth >= max
    {
        return Vec::new();
    }
    let mut children = tree.child_entries(parent);
    children.retain(|e| args.include_trashed || !e.is_trashed());
    // For tree mode, folders are kept as structure regardless of `--documents-only`.
    // `--folders-only` strips documents entirely.
    if args.folders_only {
        children.retain(|e| e.is_folder());
    }
    if args.documents_only {
        // Keep folders so users still see hierarchy; drop folders that have no
        // surviving documents in their subtree (otherwise the tree fills with
        // empty folders).
        children
            .retain(|e| e.is_document() || folder_has_descendants(tree, e, args, current_depth));
    }
    tree::sort_entries(&mut children, args.sort.as_ref());

    children
        .into_iter()
        .map(|e| TreeNode {
            uuid: Some(e.uuid),
            name: e.visible_name.clone(),
            kind: e.item_type.into(),
            file_type: e.file_type,
            last_opened: e.last_opened,
            tags: e.tags.clone(),
            pinned: e.pinned,
            children: if e.is_folder() {
                build_tree_children(tree, &Parent::Folder(e.uuid), args, current_depth + 1)
            } else {
                Vec::new()
            },
        })
        .collect()
}

fn folder_has_descendants(
    tree: &DocumentTree,
    folder: &DocumentEntry,
    args: &LsArgs,
    current_depth: u32,
) -> bool {
    if !folder.is_folder() {
        return false;
    }
    if let Some(max) = args.depth
        && current_depth + 1 >= max
    {
        return false;
    }
    for child in tree.child_entries(&Parent::Folder(folder.uuid)) {
        if !args.include_trashed && child.is_trashed() {
            continue;
        }
        if child.is_document() {
            return true;
        }
        if folder_has_descendants(tree, child, args, current_depth + 1) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn print_output(out: &LsOutput, format: OutputFormat) {
    match (out, format) {
        (LsOutput::Flat(items), OutputFormat::Json) => output::print_json(items),
        (LsOutput::Flat(items), OutputFormat::Human) => print_flat_human(items),
        (LsOutput::Tree(node), OutputFormat::Json) => output::print_json(node),
        (LsOutput::Tree(node), OutputFormat::Human) => print_tree_human(node),
    }
}

fn print_flat_human(items: &[LsItem]) {
    if items.is_empty() {
        println!("(empty)");
        return;
    }
    let recursive = items.iter().any(|i| i.depth.is_some());
    let rows: Vec<[String; 4]> = items
        .iter()
        .map(|i| {
            let type_label = type_label(i.kind, i.file_type);
            let name = if recursive {
                i.path.clone()
            } else if i.kind == ItemKind::Folder {
                format!("{}/", i.name)
            } else {
                let mut n = i.name.clone();
                if i.pinned {
                    n.push_str(" *");
                }
                n
            };
            let modified = i.modified.format("%Y-%m-%d").to_string();
            let extras = format_extras(i);
            [type_label, name, modified, extras]
        })
        .collect();

    let widths = column_widths(&["TYPE", "NAME", "MODIFIED", ""], &rows);
    println!(
        "{:<w0$}  {:<w1$}  {:<w2$}",
        "TYPE",
        "NAME",
        "MODIFIED",
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
    );
    for row in &rows {
        let extras = &row[3];
        if extras.is_empty() {
            println!(
                "{:<w0$}  {:<w1$}  {:<w2$}",
                row[0],
                row[1],
                row[2],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
            );
        } else {
            println!(
                "{:<w0$}  {:<w1$}  {:<w2$}  {extras}",
                row[0],
                row[1],
                row[2],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
            );
        }
    }
}

fn type_label(kind: ItemKind, file_type: Option<FileType>) -> String {
    match (kind, file_type) {
        (ItemKind::Folder, _) => "folder".to_string(),
        (ItemKind::Document, Some(FileType::Pdf)) => "pdf".to_string(),
        (ItemKind::Document, Some(FileType::Epub)) => "epub".to_string(),
        (ItemKind::Document, Some(FileType::Notebook)) => "notebook".to_string(),
        (ItemKind::Document, None) => "document".to_string(),
        (ItemKind::Template, _) => "template".to_string(),
    }
}

fn format_extras(item: &LsItem) -> String {
    let mut parts = Vec::new();
    if item.deleted {
        parts.push("[trashed]".to_string());
    }
    match item.kind {
        ItemKind::Folder => {
            if let Some(n) = item.children_count {
                parts.push(format!("{n} item{}", if n == 1 { "" } else { "s" }));
            }
        }
        ItemKind::Document => {
            if let Some(p) = item.page_count {
                parts.push(format!("{p}p"));
            }
        }
        ItemKind::Template => {}
    }
    if !item.tags.is_empty() {
        parts.push(format!("tags: {}", item.tags.join(", ")));
    }
    parts.join("  ")
}

fn column_widths(headers: &[&str; 4], rows: &[[String; 4]]) -> [usize; 4] {
    let mut w = [0usize; 4];
    for (i, h) in headers.iter().enumerate() {
        w[i] = h.len();
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > w[i] {
                w[i] = cell.len();
            }
        }
    }
    w
}

fn print_tree_human(root: &TreeNode) {
    println!("{}", tree_label(root, true));
    print_tree_children(&root.children, "");
}

fn print_tree_children(nodes: &[TreeNode], prefix: &str) {
    let count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let last = i == count - 1;
        let connector = if last { "└── " } else { "├── " };
        println!("{prefix}{connector}{}", tree_label(node, false));
        if !node.children.is_empty() {
            let next_prefix = if last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            print_tree_children(&node.children, &next_prefix);
        }
    }
}

fn tree_label(node: &TreeNode, is_root: bool) -> String {
    if is_root && node.uuid.is_none() {
        return node.name.clone();
    }
    let mut parts = Vec::new();
    let name = if node.kind == ItemKind::Folder {
        format!("{}/", node.name)
    } else {
        node.name.clone()
    };
    parts.push(name);
    let type_tag = match (node.kind, node.file_type) {
        (ItemKind::Folder, _) => None,
        (ItemKind::Document, Some(FileType::Pdf)) => Some("[pdf]"),
        (ItemKind::Document, Some(FileType::Epub)) => Some("[epub]"),
        (ItemKind::Document, Some(FileType::Notebook)) => Some("[notebook]"),
        (ItemKind::Document, None) => Some("[document]"),
        (ItemKind::Template, _) => Some("[template]"),
    };
    if let Some(t) = type_tag {
        parts.push(t.to_string());
    }
    if let Some(opened) = node.last_opened {
        parts.push(format!("opened: {}", opened.format("%Y-%m-%d")));
    }
    if !node.tags.is_empty() {
        parts.push(format!("tags: {}", node.tags.join(", ")));
    }
    if node.pinned {
        parts.push("pinned".to_string());
    }
    parts.join("  ")
}
