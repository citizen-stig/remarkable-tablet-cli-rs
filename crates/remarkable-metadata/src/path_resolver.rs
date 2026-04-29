use std::collections::HashSet;

use anyhow::anyhow;
use uuid::Uuid;

use crate::error::MetadataError;
use crate::metadata::{DocumentEntry, FileType, ItemKind, Parent};
use crate::tree::{ChildLookup, DocumentTree};

/// Result of resolving a path-or-UUID argument.
#[derive(Debug)]
pub enum Resolved<'a> {
    Root,
    Entry(&'a DocumentEntry),
}

const RESERVED_TRASH_PATH_MSG: &str =
    "cannot create a real path at or under /trash; trash is a virtual container";

/// Resolve a human-readable path like `"/Work/Meeting Notes"` to a tree entry.
///
/// - `"/"` resolves to `Resolved::Root`.
/// - `"/trash/<name>"` resolves within the virtual trash container.
/// - Each path segment is matched against `visible_name` at the current level.
///
/// # Errors
/// Returns an error if `path` does not start with `'/'`, a segment has no match,
/// an intermediate segment is not a folder, or two siblings share the same name.
pub fn resolve_path<'a>(tree: &'a DocumentTree, path: &str) -> anyhow::Result<Resolved<'a>> {
    if !path.starts_with('/') {
        return Err(MetadataError::InvalidPath(format!("path must start with '/': {path}")).into());
    }

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Ok(Resolved::Root);
    }

    let (mut current_parent, start_index) = if segments[0] == "trash" {
        if segments.len() == 1 {
            return Err(MetadataError::InvalidPath(
                "'/trash' is a virtual container; specify an item path under it".to_string(),
            )
            .into());
        }
        (Parent::Trash, 1)
    } else {
        (Parent::Root, 0)
    };

    for (offset, segment) in segments[start_index..].iter().enumerate() {
        let i = start_index + offset;
        let is_leaf = i == segments.len() - 1;

        let lookup = if is_leaf {
            lookup_leaf(tree, &current_parent, segment)
        } else {
            tree.lookup_child(&current_parent, segment)
        };

        match lookup {
            ChildLookup::Missing => {
                let resolved_so_far = format!("/{}", segments[..i].join("/"));
                let mut msg = format!("'{segment}' not found in {resolved_so_far}");
                let suggestions = suggest_siblings(tree, &current_parent, segment, 3);
                if !suggestions.is_empty() {
                    msg.push_str("\n  did you mean: ");
                    msg.push_str(&suggestions.join(", "));
                }
                return Err(MetadataError::NotFound(msg).into());
            }
            ChildLookup::Entry(entry) => {
                if is_leaf {
                    return Ok(Resolved::Entry(entry));
                }
                // Intermediate segments must be folders
                if !entry.is_folder() {
                    return Err(
                        MetadataError::InvalidPath(format!("'{segment}' is not a folder")).into(),
                    );
                }
                current_parent = Parent::Folder(entry.uuid);
            }
            ChildLookup::Ambiguous => {
                return Err(MetadataError::InvalidPath(format!(
                    "ambiguous: multiple items named '{segment}' in the same folder — use a UUID instead"
                ))
                .into());
            }
        }
    }

    Ok(Resolved::Root)
}

/// Look up the leaf segment, accepting `Name.<ext>` as equivalent to `Name`
/// when a sibling document's `file_type` matches that extension. Literal
/// matches always win — the extension fallback only fires on `Missing`.
fn lookup_leaf<'a>(tree: &'a DocumentTree, parent: &Parent, segment: &str) -> ChildLookup<'a> {
    let initial = tree.lookup_child(parent, segment);
    if !matches!(initial, ChildLookup::Missing) {
        return initial;
    }

    let Some((stem, ext)) = strip_known_extension(segment) else {
        return ChildLookup::Missing;
    };

    let mut matches = tree.child_entries(parent).into_iter().filter(|entry| {
        entry.visible_name == stem
            && matches!(
                &entry.kind,
                ItemKind::Document { file_type, .. } if file_type.extension() == ext,
            )
    });

    match (matches.next(), matches.next()) {
        (None, _) => ChildLookup::Missing,
        (Some(entry), None) => ChildLookup::Entry(entry),
        (Some(_), Some(_)) => ChildLookup::Ambiguous,
    }
}

/// Split a recognized document extension off `name`. Returns `(stem, ext)`
/// only when the suffix is one of `pdf`/`epub`/`rm` and the stem is non-empty.
fn strip_known_extension(name: &str) -> Option<(&str, &'static str)> {
    for ft in [FileType::Pdf, FileType::Epub, FileType::Notebook] {
        let ext = ft.extension();
        if let Some(stem) = name.strip_suffix(ext).and_then(|s| s.strip_suffix('.'))
            && !stem.is_empty()
        {
            return Some((stem, ext));
        }
    }
    None
}

/// Up to `n` sibling names that look close to `name`, formatted with their
/// extension (`Foo.pdf`) or trailing slash (`Bar/`) so the user can copy them
/// directly into a path. Substring matches sort before edit-distance matches.
fn suggest_siblings(tree: &DocumentTree, parent: &Parent, name: &str, n: usize) -> Vec<String> {
    let needle_lower = name.to_lowercase();
    let needle_stem = strip_known_extension(&needle_lower)
        .map(|(stem, _)| stem.to_string())
        .unwrap_or(needle_lower);

    let mut scored: Vec<(usize, String)> = tree
        .child_entries(parent)
        .into_iter()
        .filter_map(|entry| {
            let candidate_lower = entry.visible_name.to_lowercase();
            let score = if candidate_lower.contains(&needle_stem)
                || needle_stem.contains(&candidate_lower)
            {
                0
            } else {
                let dist = levenshtein(&candidate_lower, &needle_stem);
                if dist <= 2 { dist } else { return None }
            };
            Some((score, format_suggestion(entry)))
        })
        .collect();

    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().take(n).map(|(_, s)| s).collect()
}

fn format_suggestion(entry: &DocumentEntry) -> String {
    match &entry.kind {
        ItemKind::Document { file_type, .. } => {
            format!("{}.{}", entry.visible_name, file_type.extension())
        }
        ItemKind::Folder => format!("{}/", entry.visible_name),
        ItemKind::Template => entry.visible_name.clone(),
    }
}

/// Standard iterative Levenshtein distance with two rolling rows.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Reject writes that would materialize a real path at or under `/trash`.
/// That namespace is reserved for the virtual trash container used by path
/// resolution and tree output.
///
/// # Errors
/// Returns [`MetadataError::InvalidPath`] if `parent` + `name` would collide with
/// the reserved `/trash` namespace.
pub fn ensure_not_reserved_trash_path(
    tree: &DocumentTree,
    parent: &Parent,
    name: &str,
) -> anyhow::Result<()> {
    if would_create_reserved_trash_path(tree, parent, name) {
        return Err(MetadataError::InvalidPath(RESERVED_TRASH_PATH_MSG.to_string()).into());
    }
    Ok(())
}

fn would_create_reserved_trash_path(tree: &DocumentTree, parent: &Parent, name: &str) -> bool {
    if matches!(parent, Parent::Root) {
        return name == "trash";
    }
    if matches!(parent, Parent::Trash) {
        return false;
    }

    let mut current_parent = parent;
    let mut seen = HashSet::new();
    loop {
        match current_parent {
            Parent::Root | Parent::Trash => return false,
            Parent::Folder(uuid) => {
                if !seen.insert(*uuid) {
                    return false;
                }
                let Some(entry) = tree.get(uuid) else {
                    return false;
                };
                if entry.parent == Parent::Root {
                    return entry.visible_name == "trash";
                }
                current_parent = &entry.parent;
            }
        }
    }
}

/// Resolve a UUID to its full human-readable path (e.g., `"/Work/Meeting Notes"`).
///
/// # Errors
/// Returns an error if `uuid` is not in the tree or its parent chain is cyclic.
pub fn resolve_uuid_to_path(tree: &DocumentTree, uuid: &Uuid) -> anyhow::Result<String> {
    if tree.get(uuid).is_none() {
        return Err(MetadataError::NotFound(format!("UUID {uuid} not found")).into());
    }

    tree.display_path(uuid)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("cyclic parent chain while resolving UUID {uuid}"))
}

/// Accept either a UUID or a human path and resolve it.
///
/// Tries `Uuid::parse_str` first. If valid, looks up in the tree.
/// Otherwise treats the input as a human-readable path.
/// `"/"` always resolves to `Resolved::Root`.
///
/// # Errors
/// Returns an error if `input` is a syntactically valid UUID that is not in the tree,
/// or if it is a path that fails to resolve (see [`resolve_path`]).
pub fn resolve<'a>(tree: &'a DocumentTree, input: &str) -> anyhow::Result<Resolved<'a>> {
    if input == "/" {
        return Ok(Resolved::Root);
    }

    // Try UUID first
    if let Ok(uuid) = Uuid::parse_str(input) {
        if let Some(entry) = tree.get(&uuid) {
            return Ok(Resolved::Entry(entry));
        }
        return Err(MetadataError::NotFound(format!("UUID {uuid} not found")).into());
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
    use crate::metadata::{DocumentEntry, FileType, ItemKind, ItemType};
    use chrono::{TimeZone, Utc};

    fn make_entry(
        uuid: &str,
        name: &str,
        item_type: ItemType,
        parent: Parent,
        file_type: Option<FileType>,
    ) -> DocumentEntry {
        let deleted = parent == Parent::Trash;
        let kind = match (item_type, file_type) {
            (ItemType::Collection, _) => ItemKind::Folder,
            (ItemType::Template, _) => ItemKind::Template,
            (ItemType::Document, Some(file_type)) => ItemKind::Document {
                file_type,
                page_count: None,
            },
            (ItemType::Document, None) => {
                unreachable!("test helper requires Some(file_type) for documents")
            }
        };
        DocumentEntry {
            uuid: Uuid::parse_str(uuid).unwrap(),
            visible_name: name.to_string(),
            kind,
            parent,
            deleted,
            pinned: false,
            last_modified: Utc.timestamp_millis_opt(1_710_000_000_000).unwrap(),
            version: 1,
            tags: vec![],
            last_opened: None,
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
        assert!(err.downcast_ref::<MetadataError>().is_some());
    }

    #[test]
    fn resolve_nested_not_found() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Work/Nonexistent").unwrap_err();
        let cli_err = err.downcast_ref::<MetadataError>().unwrap();
        match cli_err {
            MetadataError::NotFound(msg) => assert!(msg.contains("Nonexistent")),
            MetadataError::InvalidPath(_) => panic!("expected NotFound, got {cli_err:?}"),
        }
    }

    #[test]
    fn resolve_path_through_document_fails() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Quick Note/child").unwrap_err();
        let cli_err = err.downcast_ref::<MetadataError>().unwrap();
        assert!(matches!(cli_err, MetadataError::InvalidPath(_)));
    }

    #[test]
    fn resolve_no_leading_slash() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "Work").unwrap_err();
        let cli_err = err.downcast_ref::<MetadataError>().unwrap();
        assert!(matches!(cli_err, MetadataError::InvalidPath(_)));
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
        let cli_err = err.downcast_ref::<MetadataError>().unwrap();
        assert!(matches!(cli_err, MetadataError::InvalidPath(_)));
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
        assert!(err.downcast_ref::<MetadataError>().is_some());
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
    fn trashed_uuid_to_path_round_trips_through_path_resolver() {
        let tree = sample_tree();
        let uuid = Uuid::parse_str(DOC_TRASH).unwrap();
        let path = resolve_uuid_to_path(&tree, &uuid).unwrap();

        match resolve_path(&tree, &path) {
            Ok(Resolved::Entry(entry)) => assert_eq!(entry.uuid, uuid),
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_trash_container_path_is_invalid() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/trash").unwrap_err();
        let cli_err = err.downcast_ref::<MetadataError>().unwrap();
        assert!(matches!(cli_err, MetadataError::InvalidPath(_)));
    }

    #[test]
    fn reserved_trash_path_rejects_root_name_trash() {
        let tree = sample_tree();
        assert!(would_create_reserved_trash_path(
            &tree,
            &Parent::Root,
            "trash"
        ));
        assert!(ensure_not_reserved_trash_path(&tree, &Parent::Root, "trash").is_err());
    }

    #[test]
    fn reserved_trash_path_rejects_real_root_trash_subtree() {
        let trash_root = Uuid::parse_str(FOLDER_A).unwrap();
        let nested = Uuid::parse_str(FOLDER_B).unwrap();
        let tree = DocumentTree::build(vec![
            make_entry(FOLDER_A, "trash", ItemType::Collection, Parent::Root, None),
            make_entry(
                FOLDER_B,
                "Nested",
                ItemType::Collection,
                Parent::Folder(trash_root),
                None,
            ),
        ]);

        assert!(would_create_reserved_trash_path(
            &tree,
            &Parent::Folder(trash_root),
            "Doc"
        ));
        assert!(would_create_reserved_trash_path(
            &tree,
            &Parent::Folder(nested),
            "Doc"
        ));
    }

    #[test]
    fn reserved_trash_path_allows_virtual_trash_parent() {
        let tree = sample_tree();
        assert!(!would_create_reserved_trash_path(
            &tree,
            &Parent::Trash,
            "Recovered"
        ));
        assert!(ensure_not_reserved_trash_path(&tree, &Parent::Trash, "Recovered").is_ok());
    }

    #[test]
    fn reserved_trash_path_allows_descendants_of_trashed_folder() {
        let trashed_folder = Uuid::parse_str(FOLDER_A).unwrap();
        let tree = DocumentTree::build(vec![
            make_entry(
                FOLDER_A,
                "Old Folder",
                ItemType::Collection,
                Parent::Trash,
                None,
            ),
            make_entry(
                DOC_1,
                "Recovered",
                ItemType::Document,
                Parent::Folder(trashed_folder),
                Some(FileType::Pdf),
            ),
        ]);

        assert!(!would_create_reserved_trash_path(
            &tree,
            &Parent::Folder(trashed_folder),
            "Recovered"
        ));
    }

    #[test]
    fn uuid_to_path_not_found() {
        let tree = sample_tree();
        let uuid = Uuid::new_v4();
        assert!(resolve_uuid_to_path(&tree, &uuid).is_err());
    }

    #[test]
    fn uuid_to_path_cycle_is_err() {
        let folder_a = Uuid::parse_str(FOLDER_A).unwrap();
        let folder_b = Uuid::parse_str(FOLDER_B).unwrap();
        let doc_uuid = Uuid::parse_str(DOC_1).unwrap();
        let tree = DocumentTree::build(vec![
            make_entry(
                FOLDER_A,
                "Folder A",
                ItemType::Collection,
                Parent::Folder(folder_b),
                None,
            ),
            make_entry(
                FOLDER_B,
                "Folder B",
                ItemType::Collection,
                Parent::Folder(folder_a),
                None,
            ),
            make_entry(
                DOC_1,
                "Looped Doc",
                ItemType::Document,
                Parent::Folder(folder_b),
                Some(FileType::Pdf),
            ),
        ]);

        assert!(resolve_uuid_to_path(&tree, &doc_uuid).is_err());
    }

    // -----------------------------------------------------------------
    // Extension-aware leaf lookup + suggestions
    // -----------------------------------------------------------------

    #[test]
    fn resolve_path_with_pdf_extension() {
        let tree = sample_tree();
        match resolve_path(&tree, "/Work/Projects/Design Doc.pdf") {
            Ok(Resolved::Entry(e)) => {
                assert_eq!(e.visible_name, "Design Doc");
                assert_eq!(e.uuid, Uuid::parse_str(DOC_2).unwrap());
            }
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_with_epub_extension() {
        let tree = DocumentTree::build(vec![make_entry(
            DOC_1,
            "Rust Book",
            ItemType::Document,
            Parent::Root,
            Some(FileType::Epub),
        )]);
        match resolve_path(&tree, "/Rust Book.epub") {
            Ok(Resolved::Entry(e)) => assert_eq!(e.uuid, Uuid::parse_str(DOC_1).unwrap()),
            other => panic!("expected Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_literal_name_takes_precedence() {
        let tree = DocumentTree::build(vec![
            make_entry(
                DOC_1,
                "Report.pdf",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Pdf),
            ),
            make_entry(
                DOC_2,
                "Report",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Pdf),
            ),
        ]);
        match resolve_path(&tree, "/Report.pdf") {
            Ok(Resolved::Entry(e)) => {
                assert_eq!(e.visible_name, "Report.pdf");
                assert_eq!(e.uuid, Uuid::parse_str(DOC_1).unwrap());
            }
            other => panic!("expected literal Entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_extension_must_match_file_type() {
        let tree = DocumentTree::build(vec![make_entry(
            DOC_1,
            "Report",
            ItemType::Document,
            Parent::Root,
            Some(FileType::Pdf),
        )]);
        let err = resolve_path(&tree, "/Report.epub").unwrap_err();
        assert!(matches!(
            err.downcast_ref::<MetadataError>().unwrap(),
            MetadataError::NotFound(_)
        ));
    }

    #[test]
    fn resolve_path_extension_disambiguates_same_name() {
        let tree = DocumentTree::build(vec![
            make_entry(
                DOC_1,
                "Notes",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Pdf),
            ),
            make_entry(
                DOC_2,
                "Notes",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Epub),
            ),
        ]);
        let bare = resolve_path(&tree, "/Notes").unwrap_err();
        assert!(matches!(
            bare.downcast_ref::<MetadataError>().unwrap(),
            MetadataError::InvalidPath(_)
        ));
        match resolve_path(&tree, "/Notes.pdf") {
            Ok(Resolved::Entry(e)) => assert_eq!(e.uuid, Uuid::parse_str(DOC_1).unwrap()),
            other => panic!("expected Pdf entry, got {other:?}"),
        }
        match resolve_path(&tree, "/Notes.epub") {
            Ok(Resolved::Entry(e)) => assert_eq!(e.uuid, Uuid::parse_str(DOC_2).unwrap()),
            other => panic!("expected Epub entry, got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_intermediate_extension_does_not_strip() {
        // Doc "Work" of Pdf type at root. /Work.pdf/anything must NotFound at the
        // intermediate segment (no stripping there) — never reach an "is not a
        // folder" error against the would-be-stripped Pdf doc.
        let tree = DocumentTree::build(vec![make_entry(
            DOC_1,
            "Work",
            ItemType::Document,
            Parent::Root,
            Some(FileType::Pdf),
        )]);
        let err = resolve_path(&tree, "/Work.pdf/anything").unwrap_err();
        assert!(matches!(
            err.downcast_ref::<MetadataError>().unwrap(),
            MetadataError::NotFound(_)
        ));
    }

    #[test]
    fn resolve_path_unknown_extension_falls_through() {
        let tree = DocumentTree::build(vec![make_entry(
            DOC_1,
            "Report",
            ItemType::Document,
            Parent::Root,
            Some(FileType::Pdf),
        )]);
        let err = resolve_path(&tree, "/Report.txt").unwrap_err();
        assert!(matches!(
            err.downcast_ref::<MetadataError>().unwrap(),
            MetadataError::NotFound(_)
        ));
    }

    #[test]
    fn not_found_error_includes_suggestions() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Work/Meting Notes").unwrap_err();
        let msg = err.downcast_ref::<MetadataError>().unwrap().to_string();
        assert!(
            msg.contains("did you mean"),
            "expected suggestion line, got: {msg}"
        );
        assert!(
            msg.contains("Meeting Notes.rm"),
            "expected close sibling with extension, got: {msg}"
        );
    }

    #[test]
    fn suggestion_formats_folders_with_trailing_slash() {
        let tree = sample_tree();
        let err = resolve_path(&tree, "/Wrk").unwrap_err();
        let msg = err.downcast_ref::<MetadataError>().unwrap().to_string();
        assert!(
            msg.contains("Work/"),
            "expected folder suggestion, got: {msg}"
        );
    }
}
