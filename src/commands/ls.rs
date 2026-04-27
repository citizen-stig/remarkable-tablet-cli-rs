use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::{GlobalOptions, LsArgs};
use crate::commands::common::{self, ItemKind};
use crate::error::{CliError, Result};
use crate::metadata::{DocumentEntry, FileType, Parent};
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tree::{self, DocumentTree, ListFilter};

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
    /// `None` for virtual nodes such as the synthetic root (`/`) or `trash`.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
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
    let filter = filter_from_args(args);
    let mut items: Vec<_> = tree
        .list_children(&target.parent, filter)
        .into_iter()
        .map(|e| to_ls_item(tree, e, None))
        .collect();

    if args.include_trashed && matches!(target.parent, Parent::Root) {
        let trash_filter = ListFilter {
            include_trashed: true,
            ..filter
        };
        items.extend(
            tree.list_children(&Parent::Trash, trash_filter)
                .into_iter()
                .map(|e| to_ls_item(tree, e, None)),
        );
    }

    items
}

fn filter_from_args(args: &LsArgs) -> ListFilter<'_> {
    ListFilter {
        include_trashed: args.include_trashed,
        documents_only: args.documents_only,
        folders_only: args.folders_only,
        sort: args.sort.as_ref(),
    }
}

fn to_ls_item(tree: &DocumentTree, e: &DocumentEntry, depth: Option<u32>) -> LsItem {
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
        parent_uuid: e.parent_uuid(),
        path: common::entry_path(tree, e),
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

fn virtual_tree_folder(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        uuid: None,
        name: name.to_string(),
        kind: ItemKind::Folder,
        file_type: None,
        last_opened: None,
        tags: Vec::new(),
        pinned: false,
        page_count: None,
        children,
    }
}

// ---------------------------------------------------------------------------
// Flat recursive listing
// ---------------------------------------------------------------------------

fn build_recursive(
    tree: &DocumentTree,
    target: &Target,
    args: &LsArgs,
) -> anyhow::Result<Vec<LsItem>> {
    let filter = filter_from_args(args);
    let mut pairs = tree.list_recursive(&target.parent, args.depth, filter)?;
    // Trashed items live under `Parent::Trash`, which is not a descendant of
    // `Parent::Root`. Walk the trash subtree explicitly when the user asked
    // for it from a root listing.
    if args.include_trashed && matches!(target.parent, Parent::Root) {
        let trash_filter = ListFilter {
            include_trashed: true,
            ..filter
        };
        pairs.extend(tree.list_recursive(&Parent::Trash, args.depth, trash_filter)?);
    }
    Ok(pairs
        .into_iter()
        .map(|(depth, e)| to_ls_item(tree, e, Some(depth)))
        .collect())
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
    if args.include_trashed && matches!(target.parent, Parent::Root) {
        let trash_children = build_tree_children(tree, &Parent::Trash, args, 0);
        if !trash_children.is_empty() {
            children.push(virtual_tree_folder("trash", trash_children));
        }
    }

    TreeNode {
        uuid,
        name: target.name.clone(),
        kind,
        file_type,
        last_opened,
        tags,
        pinned,
        page_count: None,
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
    if args.folders_only {
        children.retain(|e| e.is_folder());
    }
    tree::sort_entries(&mut children, args.sort.as_ref());

    // Recurse first, then prune empty folders bottom-up under `--documents-only`.
    // This collapses what was an O(n²) pre-filter (folder_has_descendants) into
    // a single tree walk.
    children
        .into_iter()
        .filter_map(|e| {
            let nested = if e.is_folder() {
                build_tree_children(tree, &Parent::Folder(e.uuid), args, current_depth + 1)
            } else {
                Vec::new()
            };
            if args.documents_only {
                let keep = e.is_document() || (e.is_folder() && !nested.is_empty());
                if !keep {
                    return None;
                }
            }
            Some(TreeNode {
                uuid: Some(e.uuid),
                name: e.visible_name.clone(),
                kind: e.item_type.into(),
                file_type: e.file_type,
                last_opened: e.last_opened,
                tags: e.tags.clone(),
                pinned: e.pinned,
                page_count: e.page_count,
                children: nested,
            })
        })
        .collect()
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
            let type_label = common::type_label(i.kind, i.file_type).to_string();
            let name = flat_item_name(i, recursive);
            let modified = i.modified.format("%Y-%m-%d %H:%M:%S").to_string();
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
        let extras_suffix = if extras.is_empty() {
            String::new()
        } else {
            format!("  {extras}")
        };
        println!(
            "{:<w0$}  {:<w1$}  {:<w2$}{extras_suffix}",
            row[0],
            row[1],
            row[2],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
        );
    }
}

fn flat_item_name(item: &LsItem, recursive: bool) -> String {
    if recursive {
        return item.path.clone();
    }
    if item.deleted {
        let mut name = item.path.clone();
        if item.kind == ItemKind::Folder {
            name.push('/');
        } else if item.pinned {
            name.push_str(" *");
        }
        return name;
    }
    if item.kind == ItemKind::Folder {
        format!("{}/", item.name)
    } else {
        let mut name = item.name.clone();
        if item.pinned {
            name.push_str(" *");
        }
        name
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
    if let Some(p) = node.page_count {
        parts.push(format!("{p}p"));
    }
    if let Some(opened) = node.last_opened {
        parts.push(format!("opened: {}", opened.format("%Y-%m-%d %H:%M:%S")));
    }
    if !node.tags.is_empty() {
        parts.push(format!("tags: {}", node.tags.join(", ")));
    }
    if node.pinned {
        parts.push("pinned".to_string());
    }
    parts.join("  ")
}
