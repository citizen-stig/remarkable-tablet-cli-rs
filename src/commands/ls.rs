use std::collections::HashSet;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::{GlobalOptions, LsArgs, SortField};
use crate::commands::common::{self, EntryView, ItemKind};
use crate::error::{CliError, Result};
use crate::metadata::{DocumentEntry, FileType, Parent};
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tree::{self, DocumentTree, EntryKindFilter, ListFilter};

#[derive(Serialize, Debug)]
pub struct LsItem {
    #[serde(flatten)]
    pub entry: EntryView,
    /// Number of direct children for folders; `None` for documents/templates.
    pub children_count: Option<usize>,
    /// 0 for direct children of the listed folder; only set for recursive listings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct TreeNode {
    /// `None` for virtual nodes such as the synthetic root (`/`) or `trash`.
    pub uuid: Option<Uuid>,
    pub name: String,
    #[serde(flatten)]
    pub kind: ItemKind,
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
    let filter = filter_from_args(args);

    if args.tree {
        Ok(LsOutput::Tree(build_tree_node(
            tree, &target, filter, args.depth,
        )?))
    } else if args.recursive || args.depth.is_some() {
        Ok(LsOutput::Flat(build_recursive(
            tree, &target, filter, args.depth,
        )?))
    } else {
        Ok(LsOutput::Flat(build_flat(tree, &target, filter)))
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

fn build_flat(tree: &DocumentTree, target: &Target, filter: ListFilter<'_>) -> Vec<LsItem> {
    let entries = if filter.includes_trashed() && matches!(target.parent, Parent::Root) {
        filter_flat_entries(
            merged_root_and_trash_children(tree, filter.sort_field()),
            filter,
        )
    } else {
        tree.list_children(&target.parent, filter)
    };

    entries
        .into_iter()
        .map(|e| to_ls_item(tree, e, None))
        .collect()
}

fn filter_from_args(args: &LsArgs) -> ListFilter<'_> {
    let kind = match (args.documents_only, args.folders_only) {
        (false, false) => EntryKindFilter::All,
        (true, false) => EntryKindFilter::DocumentsOnly,
        (false, true) => EntryKindFilter::FoldersOnly,
        (true, true) => unreachable!("clap enforces documents_only/folders_only conflicts"),
    };

    let filter = ListFilter::new(kind);
    let filter = if args.include_trashed {
        filter.include_trashed()
    } else {
        filter
    };
    match args.sort.as_ref() {
        Some(sort) => filter.with_sort(sort),
        None => filter,
    }
}

fn to_ls_item(tree: &DocumentTree, e: &DocumentEntry, depth: Option<u32>) -> LsItem {
    let children_count = if e.is_folder() {
        Some(tree.children_count(&Parent::Folder(e.uuid)))
    } else {
        None
    };
    LsItem {
        entry: EntryView::from_entry(tree, e),
        children_count,
        depth,
    }
}

fn virtual_tree_folder(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        uuid: None,
        name: name.to_string(),
        kind: ItemKind::Folder,
        last_opened: None,
        tags: Vec::new(),
        pinned: false,
        children,
    }
}

fn merged_root_and_trash_children<'a>(
    tree: &'a DocumentTree,
    sort: Option<&SortField>,
) -> Vec<&'a DocumentEntry> {
    let mut children = tree.child_entries(&Parent::Root);
    children.extend(tree.child_entries(&Parent::Trash));
    tree::sort_entries(&mut children, sort);
    children
}

fn filter_flat_entries<'a>(
    mut entries: Vec<&'a DocumentEntry>,
    filter: ListFilter<'_>,
) -> Vec<&'a DocumentEntry> {
    entries.retain(|entry| filter.matches(entry));
    entries
}

fn visible_children<'a>(
    tree: &'a DocumentTree,
    parent: &Parent,
    filter: ListFilter<'_>,
) -> Vec<&'a DocumentEntry> {
    let mut children = tree.child_entries(parent);
    if !filter.includes_trashed() {
        children.retain(|entry| !entry.is_trashed());
    }
    tree::sort_entries(&mut children, filter.sort_field());
    children
}

fn cycle_error(uuid: Uuid) -> anyhow::Error {
    anyhow!("cycle detected while traversing folder UUID {}", uuid)
}

fn collect_recursive_items<'a>(
    tree: &'a DocumentTree,
    entries: Vec<&'a DocumentEntry>,
    current_depth: u32,
    max_depth: Option<u32>,
    filter: ListFilter<'_>,
    ancestors: &mut HashSet<Uuid>,
    result: &mut Vec<(u32, &'a DocumentEntry)>,
) -> anyhow::Result<()> {
    if let Some(max) = max_depth
        && current_depth >= max
    {
        return Ok(());
    }

    for entry in entries {
        if filter.matches(entry) {
            result.push((current_depth, entry));
        }
        if entry.is_folder() {
            if !ancestors.insert(entry.uuid) {
                return Err(cycle_error(entry.uuid));
            }
            let children = visible_children(tree, &Parent::Folder(entry.uuid), filter);
            collect_recursive_items(
                tree,
                children,
                current_depth + 1,
                max_depth,
                filter,
                ancestors,
                result,
            )?;
            ancestors.remove(&entry.uuid);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Flat recursive listing
// ---------------------------------------------------------------------------

fn build_recursive(
    tree: &DocumentTree,
    target: &Target,
    filter: ListFilter<'_>,
    depth: Option<u32>,
) -> anyhow::Result<Vec<LsItem>> {
    if filter.includes_trashed() && matches!(target.parent, Parent::Root) {
        let mut pairs = Vec::new();
        let mut ancestors = HashSet::new();
        collect_recursive_items(
            tree,
            merged_root_and_trash_children(tree, filter.sort_field()),
            0,
            depth,
            filter,
            &mut ancestors,
            &mut pairs,
        )?;
        return Ok(pairs
            .into_iter()
            .map(|(depth, e)| to_ls_item(tree, e, Some(depth)))
            .collect());
    }
    Ok(tree
        .list_recursive(&target.parent, depth, filter)?
        .into_iter()
        .map(|(depth, e)| to_ls_item(tree, e, Some(depth)))
        .collect())
}

// ---------------------------------------------------------------------------
// Tree listing
// ---------------------------------------------------------------------------

fn build_tree_node(
    tree: &DocumentTree,
    target: &Target,
    filter: ListFilter<'_>,
    depth: Option<u32>,
) -> anyhow::Result<TreeNode> {
    let (uuid, last_opened, tags, pinned) = match &target.parent {
        Parent::Folder(u) => {
            let entry = tree.get(u).expect("target folder exists in tree");
            (
                Some(entry.uuid),
                entry.last_opened,
                entry.tags.clone(),
                entry.pinned,
            )
        }
        _ => (None, None, Vec::new(), false),
    };

    let mut ancestors = HashSet::new();
    if let Parent::Folder(uuid) = target.parent {
        ancestors.insert(uuid);
    }
    let mut children = build_tree_children(tree, &target.parent, filter, depth, 0, &mut ancestors)?;
    if filter.includes_trashed() && matches!(target.parent, Parent::Root) {
        let trash_children =
            build_tree_children(tree, &Parent::Trash, filter, depth, 0, &mut ancestors)?;
        if !trash_children.is_empty() {
            children.push(virtual_tree_folder("trash", trash_children));
        }
    }

    Ok(TreeNode {
        uuid,
        name: target.name.clone(),
        kind: ItemKind::Folder,
        last_opened,
        tags,
        pinned,
        children,
    })
}

fn build_tree_children(
    tree: &DocumentTree,
    parent: &Parent,
    filter: ListFilter<'_>,
    max_depth: Option<u32>,
    current_depth: u32,
    ancestors: &mut HashSet<Uuid>,
) -> anyhow::Result<Vec<TreeNode>> {
    if let Some(max) = max_depth
        && current_depth >= max
    {
        return Ok(Vec::new());
    }
    let mut children = visible_children(tree, parent, filter);
    children.retain(|entry| filter.matches(entry) || entry.is_folder());

    // Recurse first, then prune empty folders bottom-up under `--documents-only`.
    // This collapses what was an O(n²) pre-filter (folder_has_descendants) into
    // a single tree walk.
    children
        .into_iter()
        .map(|e| {
            let nested = if e.is_folder() {
                if !ancestors.insert(e.uuid) {
                    return Err(cycle_error(e.uuid));
                }
                let nested = build_tree_children(
                    tree,
                    &Parent::Folder(e.uuid),
                    filter,
                    max_depth,
                    current_depth + 1,
                    ancestors,
                )?;
                ancestors.remove(&e.uuid);
                nested
            } else {
                Vec::new()
            };
            if !filter.matches(e) {
                if e.is_folder() && !nested.is_empty() {
                    // Keep ancestor folders in tree mode when documents are selected.
                } else {
                    return Ok(None);
                }
            }
            Ok(Some(TreeNode {
                uuid: Some(e.uuid),
                name: e.visible_name.clone(),
                kind: e.kind.clone(),
                last_opened: e.last_opened,
                tags: e.tags.clone(),
                pinned: e.pinned,
                children: nested,
            }))
        })
        .filter_map(|node| node.transpose())
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
            let type_label = common::type_label(&i.entry.kind).to_string();
            let name = flat_item_name(i, recursive);
            let modified = i.entry.last_modified.format("%Y-%m-%d %H:%M:%S").to_string();
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
    let e = &item.entry;
    if recursive {
        return e.path.clone();
    }
    if e.deleted {
        let mut name = e.path.clone();
        if matches!(e.kind, ItemKind::Folder) {
            name.push('/');
        } else if e.pinned {
            name.push_str(" *");
        }
        return name;
    }
    if matches!(e.kind, ItemKind::Folder) {
        format!("{}/", e.name)
    } else {
        let mut name = e.name.clone();
        if e.pinned {
            name.push_str(" *");
        }
        name
    }
}

fn format_extras(item: &LsItem) -> String {
    let e = &item.entry;
    let mut parts = Vec::new();
    if e.deleted {
        parts.push("[trashed]".to_string());
    }
    match &e.kind {
        ItemKind::Folder => {
            if let Some(n) = item.children_count {
                parts.push(format!("{n} item{}", if n == 1 { "" } else { "s" }));
            }
        }
        ItemKind::Document {
            page_count: Some(p),
            ..
        } => {
            parts.push(format!("{p}p"));
        }
        ItemKind::Document { .. } | ItemKind::Template => {}
    }
    if !e.tags.is_empty() {
        parts.push(format!("tags: {}", e.tags.join(", ")));
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
    let name = if matches!(node.kind, ItemKind::Folder) {
        format!("{}/", node.name)
    } else {
        node.name.clone()
    };
    parts.push(name);
    let type_tag = match &node.kind {
        ItemKind::Folder => None,
        ItemKind::Document {
            file_type: FileType::Pdf,
            ..
        } => Some("[pdf]"),
        ItemKind::Document {
            file_type: FileType::Epub,
            ..
        } => Some("[epub]"),
        ItemKind::Document {
            file_type: FileType::Notebook,
            ..
        } => Some("[notebook]"),
        ItemKind::Template => Some("[template]"),
    };
    if let Some(t) = type_tag {
        parts.push(t.to_string());
    }
    if let ItemKind::Document {
        page_count: Some(p),
        ..
    } = &node.kind
    {
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
