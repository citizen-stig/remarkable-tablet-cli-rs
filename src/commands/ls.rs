use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::LsArgs;
use crate::commands::common::{self, CommandContext, EntryView, ItemKind};
use crate::error::CliError;
use crate::metadata::{DocumentEntry, FileType, Parent};
use crate::output::{self, OutputFormat};
use crate::path_resolver::{self, Resolved};
use crate::tree::{DocumentTree, ListFilter};

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

// serde's `skip_serializing_if` predicate requires `&T` by value contract.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug)]
pub enum LsOutput {
    Flat(Vec<LsItem>),
    Tree(TreeNode),
}

/// # Errors
/// Returns an error if connection fails, metadata cannot be loaded, or the path/UUID does not resolve.
pub async fn execute(ctx: &CommandContext, args: &LsArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &LsArgs) -> anyhow::Result<()> {
    let (session, tree) = ctx.connect_and_load_tree().await?;
    let result = run_with_tree(&tree, args);
    session.ssh.disconnect().await;
    let output = result?;
    print_output(&output, ctx.format());
    Ok(())
}

/// # Errors
/// Returns an error if `args.path_or_uuid` does not resolve, or if a folder cycle is detected during recursive traversal.
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
    tree.list_children(&target.parent, filter)
        .into_iter()
        .map(|e| to_ls_item(tree, e, None))
        .collect()
}

fn filter_from_args(args: &LsArgs) -> ListFilter<'_> {
    let filter = ListFilter::new(args.kind);
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

// ---------------------------------------------------------------------------
// Flat recursive listing
// ---------------------------------------------------------------------------

fn build_recursive(
    tree: &DocumentTree,
    target: &Target,
    filter: ListFilter<'_>,
    depth: Option<u32>,
) -> anyhow::Result<Vec<LsItem>> {
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

#[allow(clippy::redundant_closure_for_method_calls)] // Result::transpose as fn ptr fails E inference here
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
    let mut children = tree.sorted_direct_children(parent, filter);
    children.retain(|entry| filter.matches(entry) || entry.is_folder());

    // Recurse first, then prune empty folders bottom-up under `--documents-only`.
    // This collapses what was an O(n²) pre-filter (folder_has_descendants) into
    // a single tree walk.
    children
        .into_iter()
        .map(|e| {
            let nested = if e.is_folder() {
                if !ancestors.insert(e.uuid) {
                    return Err(anyhow::anyhow!(
                        "cycle detected while traversing folder UUID {}",
                        e.uuid
                    ));
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
    println!("{}", format_output(out, format));
}

/// Render `out` to a string in the requested format. Used by `print_output`
/// and by snapshot tests.
///
/// # Panics
/// Panics if `out` cannot be serialized as JSON.
#[must_use]
pub fn format_output(out: &LsOutput, format: OutputFormat) -> String {
    match (out, format) {
        (LsOutput::Flat(items), OutputFormat::Json) => output::render_json(items),
        (LsOutput::Flat(items), OutputFormat::Human) => format_flat_human(items),
        (LsOutput::Tree(node), OutputFormat::Json) => output::render_json(node),
        (LsOutput::Tree(node), OutputFormat::Human) => format_tree_human(node),
    }
}

fn format_flat_human(items: &[LsItem]) -> String {
    if items.is_empty() {
        return "(empty)".to_string();
    }
    let recursive = items.iter().any(|i| i.depth.is_some());
    let rows: Vec<[String; 4]> = items
        .iter()
        .map(|i| {
            let type_label = common::type_label(&i.entry.kind).to_string();
            let name = flat_item_name(i, recursive);
            let modified = i
                .entry
                .last_modified
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let extras = format_extras(i);
            [type_label, name, modified, extras]
        })
        .collect();

    let widths = column_widths(&["TYPE", "NAME", "MODIFIED", ""], &rows);
    let mut lines = vec![format!(
        "{:<w0$}  {:<w1$}  {:<w2$}",
        "TYPE",
        "NAME",
        "MODIFIED",
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
    )];
    for row in &rows {
        let extras = &row[3];
        let extras_suffix = if extras.is_empty() {
            String::new()
        } else {
            format!("  {extras}")
        };
        lines.push(format!(
            "{:<w0$}  {:<w1$}  {:<w2$}{extras_suffix}",
            row[0],
            row[1],
            row[2],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
        ));
    }
    lines.join("\n")
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

fn format_tree_human(root: &TreeNode) -> String {
    let mut lines = vec![tree_label(root, true)];
    push_tree_children(&mut lines, &root.children, "");
    lines.join("\n")
}

fn push_tree_children(lines: &mut Vec<String>, nodes: &[TreeNode], prefix: &str) {
    let count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let last = i == count - 1;
        let connector = if last { "└── " } else { "├── " };
        lines.push(format!("{prefix}{connector}{}", tree_label(node, false)));
        if !node.children.is_empty() {
            let next_prefix = if last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            push_tree_children(lines, &node.children, &next_prefix);
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
