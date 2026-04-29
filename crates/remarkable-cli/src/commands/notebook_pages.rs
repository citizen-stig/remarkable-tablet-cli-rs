//! Shared helpers for enumerating a notebook's `.rm` page files.
//!
//! Both `download` and `render` need to:
//! 1. List the `<uuid>/` directory on the source.
//! 2. Read `<uuid>.content` to recover the user-visible page order.
//! 3. Optionally apply a 1-indexed `--pages` selection.
//!
//! The helpers here keep that logic in one place. `order_page_files_*`
//! mutate their `discovered` argument (sorting it) so callers don't pay
//! for an extra clone — the same shape `download.rs` already used before
//! the split.
//!
//! The page-source-side I/O (SFTP for the tablet vs local fs for a
//! backup) stays with each command; only the pure ordering logic lives
//! here.

use std::collections::HashSet;

use anyhow::{Context, bail};

use remarkable_metadata::metadata;

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
    let content = metadata::parse_content(content_bytes).context("parse notebook .content")?;
    let Some(pages) = content.pages.as_deref() else {
        bail!("notebook .content is missing a pages array; --pages requires recorded page order");
    };
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
    discovered.sort();
    let Some(arr) = pages else {
        return std::mem::take(discovered);
    };

    let mut ordered = Vec::with_capacity(discovered.len());
    let mut remaining: HashSet<String> = discovered.iter().cloned().collect();
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
#[must_use]
pub fn page_count_zero(content_bytes: Option<&[u8]>) -> bool {
    let Some(bytes) = content_bytes else {
        return false;
    };
    let Ok(content) = metadata::parse_content(bytes) else {
        return false;
    };
    content.effective_page_count() == Some(0)
}
