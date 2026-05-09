//! Shared helpers for enumerating a notebook's `.rm` page files.
//!
//! `list_selected_pages` runs the full pipeline (list `<uuid>/`, read
//! `<uuid>.content`, optionally apply a 1-indexed `--pages` selection)
//! against any [`TabletConnection`]. The lower-level `order_page_files_*`
//! functions are exposed for callers that only need the ordering step.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, anyhow};

use remarkable_metadata::metadata::{self, DocumentEntry, RawContent};
use remarkable_metadata::page_range::PageSelection;
use remarkable_tablet::connection::TabletConnection;

/// Resolve the page files for a notebook, applying the user's `--pages`
/// selection if any. The returned list is `(1-based page index,
/// filename)` pairs in render order; `pages = None` returns every page.
///
/// # Errors
/// Returns an error if `--pages` is set but `.content` is unreadable, if
/// the page directory is unreadable for any reason other than "doesn't
/// exist on a zero-page notebook", or if `.content` parsing fails.
pub async fn list_selected_pages<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    entry: &DocumentEntry,
    pages: Option<&PageSelection>,
) -> anyhow::Result<Vec<(u32, String)>> {
    let pages_dir = format!("{data_dir}/{}", entry.uuid);
    let content_path = format!("{data_dir}/{}.content", entry.uuid);

    // Page-dir listing and `.content` are independent reads; running them
    // concurrently saves one round-trip on a slow tablet link.
    let (entries_res, content_res) =
        tokio::join!(conn.read_dir(&pages_dir), conn.read_file(&content_path));
    let content_bytes = match content_res {
        Ok(bytes) => Some(bytes),
        Err(err) if pages.is_some() => {
            return Err(err).with_context(|| {
                format!(
                    "readable notebook page order required from {content_path} when using --pages"
                )
            });
        }
        Err(_) => None,
    };
    let entries = match entries_res {
        Ok(entries) => entries,
        Err(err) => {
            if can_treat_missing_page_dir_as_empty(conn, &pages_dir, content_bytes.as_deref())
                .await?
            {
                Vec::new()
            } else {
                return Err(err).with_context(|| format!("list {pages_dir}"));
            }
        }
    };

    let mut page_files: Vec<String> = entries
        .into_iter()
        .filter(|e| {
            Path::new(&e.name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rm"))
        })
        .map(|e| e.name)
        .collect();

    let selected = match pages {
        Some(selection) => select_page_files_strict(
            content_bytes.as_deref().ok_or_else(|| {
                anyhow!(
                    "readable notebook page order required from {content_path} when using --pages"
                )
            })?,
            &mut page_files,
            selection,
        )?,
        None => enumerate_page_files(order_page_files_best_effort(
            content_bytes.as_deref(),
            &mut page_files,
        )),
    };

    Ok(selected)
}

/// Best-effort page ordering for full notebook downloads or renders: try
/// to recover the order from `.content`, otherwise fall back to sorted
/// filenames.
pub fn order_page_files_best_effort(
    content_bytes: Option<&[u8]>,
    discovered: &mut Vec<String>,
) -> Vec<String> {
    let pages = content_bytes
        .and_then(|bytes| metadata::parse_content(bytes).ok())
        .and_then(|content| content.pages);
    order_page_files_from_pages(pages.as_deref(), discovered)
}

/// Strict page ordering for `--pages` callers: requires readable
/// `.content` bytes that include a `pages` array.
pub fn order_page_files_strict(
    content_bytes: &[u8],
    discovered: &mut Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let content = parse_strict_content(content_bytes)?;
    let pages = require_pages_array(&content)?;
    Ok(order_page_files_from_pages(Some(pages), discovered))
}

/// Build a page list from the known `pages` ordering. The returned vector
/// contains every discovered `.rm` filename — any pages listed in
/// `.content` but missing on disk are silently dropped, and any orphan
/// `.rm` files not referenced by `.content` are appended at the end.
pub fn order_page_files_from_pages(
    pages: Option<&[serde_json::Value]>,
    discovered: &mut Vec<String>,
) -> Vec<String> {
    let Some(arr) = pages else {
        // Fallback path: no recorded order, hand back filename-sorted.
        discovered.sort();
        return std::mem::take(discovered);
    };

    let mut ordered = Vec::with_capacity(discovered.len());
    let mut remaining: HashSet<String> = discovered.drain(..).collect();
    for item in arr {
        let Some(page_id) = extract_page_id(item) else {
            continue;
        };
        let filename = format!("{page_id}.rm");
        if remaining.remove(&filename) {
            ordered.push(filename);
        }
    }
    let mut leftover: Vec<String> = remaining.into_iter().collect();
    leftover.sort();
    ordered.extend(leftover);
    ordered
}

fn select_page_files_strict(
    content_bytes: &[u8],
    discovered: &mut Vec<String>,
    selection: &PageSelection,
) -> anyhow::Result<Vec<(u32, String)>> {
    let content = parse_strict_content(content_bytes)?;
    let pages = require_pages_array(&content)?;
    Ok(select_page_files_from_pages(pages, discovered)
        .into_iter()
        .filter(|(page_number, _)| selection.contains(*page_number))
        .collect())
}

fn select_page_files_from_pages(
    pages: &[serde_json::Value],
    discovered: &mut Vec<String>,
) -> Vec<(u32, String)> {
    // `discovered` is drained into a `HashSet` below, so its order doesn't
    // need to be deterministic here — leftovers get sorted by filename
    // before being numbered after the recorded pages.
    let mut ordered = Vec::with_capacity(discovered.len());
    let mut remaining: HashSet<String> = discovered.drain(..).collect();
    for (idx, item) in pages.iter().enumerate() {
        let Some(page_number) = one_based_page_number(idx) else {
            continue;
        };
        let Some(page_id) = extract_page_id(item) else {
            continue;
        };
        let filename = format!("{page_id}.rm");
        if remaining.remove(&filename) {
            ordered.push((page_number, filename));
        }
    }

    let mut leftover: Vec<String> = remaining.into_iter().collect();
    leftover.sort();
    let offset = pages.len();
    ordered.extend(leftover.into_iter().enumerate().filter_map(|(idx, name)| {
        let page_number = one_based_page_number(offset + idx)?;
        Some((page_number, name))
    }));
    ordered
}

fn enumerate_page_files(ordered: Vec<String>) -> Vec<(u32, String)> {
    ordered
        .into_iter()
        .enumerate()
        .filter_map(|(idx, name)| Some((one_based_page_number(idx)?, name)))
        .collect()
}

fn parse_strict_content(content_bytes: &[u8]) -> anyhow::Result<RawContent> {
    metadata::parse_content(content_bytes).context("parse notebook .content")
}

fn require_pages_array(content: &RawContent) -> anyhow::Result<&[serde_json::Value]> {
    content.pages.as_deref().ok_or_else(|| {
        anyhow!("notebook .content is missing a pages array; --pages requires recorded page order")
    })
}

fn one_based_page_number(index: usize) -> Option<u32> {
    u32::try_from(index + 1).ok()
}

pub fn extract_page_id(item: &serde_json::Value) -> Option<&str> {
    if let Some(s) = item.as_str() {
        return Some(s);
    }
    let obj = item.as_object()?;
    obj.get("id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| obj.get("uuid").and_then(serde_json::Value::as_str))
}

/// True when a missing page directory can be treated as an empty notebook
/// — i.e. the `.content` file (if any) records zero pages. Used to keep
/// the no-content-no-pages happy path from erroring out.
async fn can_treat_missing_page_dir_as_empty<C: TabletConnection>(
    conn: &C,
    pages_dir: &str,
    content_bytes: Option<&[u8]>,
) -> anyhow::Result<bool> {
    if conn
        .file_exists(pages_dir)
        .await
        .with_context(|| format!("stat {pages_dir}"))?
    {
        return Ok(false);
    }
    Ok(page_count_zero(content_bytes))
}

fn page_count_zero(content_bytes: Option<&[u8]>) -> bool {
    let Some(bytes) = content_bytes else {
        return false;
    };
    let Ok(content) = metadata::parse_content(bytes) else {
        return false;
    };
    content.effective_page_count() == Some(0)
}
