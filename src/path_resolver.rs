use anyhow::anyhow;
use uuid::Uuid;

use crate::error::CliError;
use crate::metadata::{DocumentEntry, Parent};
use crate::tree::DocumentTree;

/// Result of resolving a path-or-UUID argument.
#[derive(Debug)]
pub enum Resolved<'a> {
    Root,
    Entry(&'a DocumentEntry),
}

/// Resolve a human-readable path like `"/Work/Meeting Notes"` to a tree entry.
///
/// - `"/"` resolves to `Resolved::Root`.
/// - Each path segment is matched against `visible_name` at the current level.
/// - Returns `CliError::NotFound` if a segment has no match.
/// - Returns an error if duplicate `visible_name` values exist at the same level.
pub fn resolve_path<'a>(tree: &'a DocumentTree, path: &str) -> anyhow::Result<Resolved<'a>> {
    if !path.starts_with('/') {
        return Err(CliError::InvalidPath(format!(
            "path must start with '/': {path}"
        ))
        .into());
    }

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Ok(Resolved::Root);
    }

    let mut current_parent = Parent::Root;

    for (i, segment) in segments.iter().enumerate() {
        let children = tree.child_entries(&current_parent);
        let matches: Vec<_> = children
            .iter()
            .filter(|e| e.visible_name == *segment)
            .collect();

        match matches.len() {
            0 => {
                let resolved_so_far = format!("/{}", segments[..i].join("/"));
                return Err(CliError::NotFound(format!(
                    "'{segment}' not found in {resolved_so_far}"
                ))
                .into());
            }
            1 => {
                let entry = matches[0];
                if i == segments.len() - 1 {
                    return Ok(Resolved::Entry(entry));
                }
                // Intermediate segments must be folders
                if !entry.is_folder() {
                    return Err(CliError::InvalidPath(format!(
                        "'{segment}' is not a folder"
                    ))
                    .into());
                }
                current_parent = Parent::Folder(entry.uuid);
            }
            _ => {
                return Err(CliError::InvalidPath(format!(
                    "ambiguous: multiple items named '{segment}' in the same folder — use a UUID instead"
                ))
                .into());
            }
        }
    }

    Ok(Resolved::Root)
}

/// Resolve a UUID to its full human-readable path (e.g., `"/Work/Meeting Notes"`).
///
/// Walks the parent chain upward. Returns `"/trash/..."` for trashed items.
/// Caps at 100 levels to guard against cycles.
pub fn resolve_uuid_to_path(tree: &DocumentTree, uuid: &Uuid) -> anyhow::Result<String> {
    let mut parts = Vec::new();
    let mut current = tree
        .get(uuid)
        .ok_or_else(|| CliError::NotFound(format!("UUID {uuid} not found")))?;

    for _ in 0..100 {
        parts.push(current.visible_name.as_str());
        match &current.parent {
            Parent::Root => {
                parts.reverse();
                return Ok(format!("/{}", parts.join("/")));
            }
            Parent::Trash => {
                parts.reverse();
                return Ok(format!("/trash/{}", parts.join("/")));
            }
            Parent::Folder(parent_uuid) => {
                current = tree.get(parent_uuid).ok_or_else(|| {
                    anyhow!("broken parent chain: UUID {parent_uuid} not found")
                })?;
            }
        }
    }

    Err(anyhow!("parent chain too deep (>100 levels), possible cycle"))
}

/// Accept either a UUID or a human path and resolve it.
///
/// Tries `Uuid::parse_str` first. If valid, looks up in the tree.
/// Otherwise treats the input as a human-readable path.
/// `"/"` always resolves to `Resolved::Root`.
pub fn resolve<'a>(tree: &'a DocumentTree, input: &str) -> anyhow::Result<Resolved<'a>> {
    if input == "/" {
        return Ok(Resolved::Root);
    }

    // Try UUID first
    if let Ok(uuid) = Uuid::parse_str(input) {
        if let Some(entry) = tree.get(&uuid) {
            return Ok(Resolved::Entry(entry));
        }
        return Err(CliError::NotFound(format!("UUID {uuid} not found")).into());
    }

    // Fall back to path resolution
    resolve_path(tree, input)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{DocumentEntry, FileType, ItemType};
    use chrono::{TimeZone, Utc};

    fn make_entry(
        uuid: &str,
        name: &str,
        item_type: ItemType,
        parent: Parent,
        file_type: Option<FileType>,
    ) -> DocumentEntry {
        let deleted = parent == Parent::Trash;
        DocumentEntry {
            uuid: Uuid::parse_str(uuid).unwrap(),
            visible_name: name.to_string(),
            item_type,
            parent,
            deleted,
            pinned: false,
            last_modified: Utc.timestamp_millis_opt(1710000000000).unwrap(),
            version: 1,
            tags: vec![],
            last_opened: None,
            file_type,
        }
    }

    const FOLDER_A: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const FOLDER_B: &str = "aaaaaaaa-0000-0000-0000-000000000002";
    const DOC_1: &str = "bbbbbbbb-0000-0000-0000-000000000001";
    const DOC_2: &str = "bbbbbbbb-0000-0000-0000-000000000002";
    const DOC_3: &str = "bbbbbbbb-0000-0000-0000-000000000003";
    const DOC_TRASH: &str = "cccccccc-0000-0000-0000-000000000001";

    fn sample_tree() -> DocumentTree {
        let folder_a = Uuid::parse_str(FOLDER_A).unwrap();
        let folder_b = Uuid::parse_str(FOLDER_B).unwrap();
        DocumentTree::build(vec![
            make_entry(FOLDER_A, "Work", ItemType::Collection, Parent::Root, None),
            make_entry(
                FOLDER_B,
                "Projects",
                ItemType::Collection,
                Parent::Folder(folder_a),
                None,
            ),
            make_entry(
                DOC_1,
                "Meeting Notes",
                ItemType::Document,
                Parent::Folder(folder_a),
                Some(FileType::Notebook),
            ),
            make_entry(
                DOC_2,
                "Design Doc",
                ItemType::Document,
                Parent::Folder(folder_b),
                Some(FileType::Pdf),
            ),
            make_entry(
                DOC_3,
                "Quick Note",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Notebook),
            ),
            make_entry(
                DOC_TRASH,
                "Old Draft",
                ItemType::Document,
                Parent::Trash,
                Some(FileType::Pdf),
            ),
        ])
    }

    #[test]
    fn resolve_root() {
        let tree = sample_tree();
        assert!(matches!(resolve_path(&tree, "/"), Ok(Resolved::Root)));
        assert!(matches!(resolve(&tree, "/"), Ok(Resolved::Root)));
    }

    #[test]
    fn resolve_root_folder() {
        let tree = sample_tree();
        match resolve_path(&tree, "/Work") {
            Ok(Resolved::Entry(e)) => assert_eq!(e.visible_name, "Work"),
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_nested_doc() {
        let tree = sample_tree();
        match resolve_path(&tree, "/Work/Meeting Notes") {
            Ok(Resolved::Entry(e)) => {
                assert_eq!(e.visible_name, "Meeting Notes");
                assert_eq!(e.uuid, Uuid::parse_str(DOC_1).unwrap());
            }
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_deeply_nested() {
        let tree = sample_tree();
        match resolve_path(&tree, "/Work/Projects/Design Doc") {
            Ok(Resolved::Entry(e)) => assert_eq!(e.visible_name, "Design Doc"),
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_not_found() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Nonexistent").unwrap_err();
        assert!(err.downcast_ref::<CliError>().is_some());
    }

    #[test]
    fn resolve_nested_not_found() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Work/Nonexistent").unwrap_err();
        let cli_err = err.downcast_ref::<CliError>().unwrap();
        match cli_err {
            CliError::NotFound(msg) => assert!(msg.contains("Nonexistent")),
            _ => panic!("expected NotFound, got {cli_err:?}"),
        }
    }

    #[test]
    fn resolve_path_through_document_fails() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Quick Note/child").unwrap_err();
        let cli_err = err.downcast_ref::<CliError>().unwrap();
        assert!(matches!(cli_err, CliError::InvalidPath(_)));
    }

    #[test]
    fn resolve_no_leading_slash() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "Work").unwrap_err();
        let cli_err = err.downcast_ref::<CliError>().unwrap();
        assert!(matches!(cli_err, CliError::InvalidPath(_)));
    }

    #[test]
    fn resolve_duplicate_names() {
        let entries = vec![
            make_entry(
                "aaaaaaaa-0000-0000-0000-000000000001",
                "Same Name",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Pdf),
            ),
            make_entry(
                "aaaaaaaa-0000-0000-0000-000000000002",
                "Same Name",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Pdf),
            ),
        ];
        let tree = DocumentTree::build(entries);
        let err = resolve_path(&tree, "/Same Name").unwrap_err();
        let cli_err = err.downcast_ref::<CliError>().unwrap();
        assert!(matches!(cli_err, CliError::InvalidPath(_)));
    }

    #[test]
    fn resolve_by_uuid() {
        let tree = sample_tree();
        match resolve(&tree, DOC_1) {
            Ok(Resolved::Entry(e)) => assert_eq!(e.visible_name, "Meeting Notes"),
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_by_uuid_not_found() {
        let tree = sample_tree();
        let err = resolve(&tree, "99999999-9999-9999-9999-999999999999").unwrap_err();
        assert!(err.downcast_ref::<CliError>().is_some());
    }

    #[test]
    fn resolve_by_path_fallback() {
        let tree = sample_tree();
        match resolve(&tree, "/Work") {
            Ok(Resolved::Entry(e)) => assert_eq!(e.visible_name, "Work"),
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn uuid_to_path_root_child() {
        let tree = sample_tree();
        let uuid = Uuid::parse_str(FOLDER_A).unwrap();
        assert_eq!(resolve_uuid_to_path(&tree, &uuid).unwrap(), "/Work");
    }

    #[test]
    fn uuid_to_path_nested() {
        let tree = sample_tree();
        let uuid = Uuid::parse_str(DOC_1).unwrap();
        assert_eq!(
            resolve_uuid_to_path(&tree, &uuid).unwrap(),
            "/Work/Meeting Notes"
        );
    }

    #[test]
    fn uuid_to_path_deeply_nested() {
        let tree = sample_tree();
        let uuid = Uuid::parse_str(DOC_2).unwrap();
        assert_eq!(
            resolve_uuid_to_path(&tree, &uuid).unwrap(),
            "/Work/Projects/Design Doc"
        );
    }

    #[test]
    fn uuid_to_path_trashed() {
        let tree = sample_tree();
        let uuid = Uuid::parse_str(DOC_TRASH).unwrap();
        assert_eq!(
            resolve_uuid_to_path(&tree, &uuid).unwrap(),
            "/trash/Old Draft"
        );
    }

    #[test]
    fn uuid_to_path_not_found() {
        let tree = sample_tree();
        let uuid = Uuid::new_v4();
        assert!(resolve_uuid_to_path(&tree, &uuid).is_err());
    }
}
