use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use uuid::Uuid;

use crate::cli::SortField;
use crate::metadata::{DocumentEntry, Parent};

/// In-memory document tree built from flat metadata entries.
///
/// Stores all entries by UUID and maintains a parent-to-children index
/// for efficient traversal.
pub struct DocumentTree {
    entries: HashMap<Uuid, DocumentEntry>,
    children: HashMap<Parent, Vec<Uuid>>,
}

impl DocumentTree {
    /// Build a tree from a flat list of document entries.
    pub fn build(entries: Vec<DocumentEntry>) -> Self {
        let mut by_uuid = HashMap::with_capacity(entries.len());
        let mut children: HashMap<Parent, Vec<Uuid>> = HashMap::new();

        for entry in entries {
            children
                .entry(entry.parent.clone())
                .or_default()
                .push(entry.uuid);
            by_uuid.insert(entry.uuid, entry);
        }

        Self {
            entries: by_uuid,
            children,
        }
    }

    /// Look up a single entry by UUID.
    pub fn get(&self, uuid: &Uuid) -> Option<&DocumentEntry> {
        self.entries.get(uuid)
    }

    /// Iterate over all entries (unordered).
    pub fn all_entries(&self) -> impl Iterator<Item = &DocumentEntry> {
        self.entries.values()
    }

    /// Direct children of the given parent as `DocumentEntry` references.
    pub fn child_entries(&self, parent: &Parent) -> Vec<&DocumentEntry> {
        self.children
            .get(parent)
            .map(|uuids| uuids.iter().filter_map(|u| self.entries.get(u)).collect())
            .unwrap_or_default()
    }

    /// Number of direct children.
    pub fn children_count(&self, parent: &Parent) -> usize {
        self.children.get(parent).map(|v| v.len()).unwrap_or(0)
    }

    /// List children of a folder with filters and sorting applied.
    pub fn list_children(
        &self,
        parent: &Parent,
        include_trashed: bool,
        documents_only: bool,
        folders_only: bool,
        sort: Option<&SortField>,
    ) -> Vec<&DocumentEntry> {
        let mut result = self.child_entries(parent);

        if !include_trashed {
            result.retain(|e| !e.is_trashed());
        }
        if documents_only {
            result.retain(|e| e.is_document());
        }
        if folders_only {
            result.retain(|e| e.is_folder());
        }

        sort_entries(&mut result, sort);
        result
    }

    /// Recursively list all descendants up to a given depth.
    ///
    /// Returns `(depth_level, entry)` pairs. `depth = None` means unlimited.
    /// `depth = Some(1)` means direct children only.
    ///
    /// Returns an error when traversal encounters a parent cycle.
    pub fn list_recursive(
        &self,
        parent: &Parent,
        depth: Option<u32>,
        include_trashed: bool,
        documents_only: bool,
        folders_only: bool,
        sort: Option<&SortField>,
    ) -> anyhow::Result<Vec<(u32, &DocumentEntry)>> {
        let mut result = Vec::new();
        let mut ancestors = HashSet::new();
        if let Parent::Folder(uuid) = parent {
            ancestors.insert(*uuid);
        }
        self.collect_recursive(
            parent,
            0,
            depth,
            include_trashed,
            documents_only,
            folders_only,
            sort,
            &mut ancestors,
            &mut result,
        )?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_recursive<'a>(
        &'a self,
        parent: &Parent,
        current_depth: u32,
        max_depth: Option<u32>,
        include_trashed: bool,
        documents_only: bool,
        folders_only: bool,
        sort: Option<&SortField>,
        ancestors: &mut HashSet<Uuid>,
        result: &mut Vec<(u32, &'a DocumentEntry)>,
    ) -> anyhow::Result<()> {
        if let Some(max) = max_depth
            && current_depth >= max
        {
            return Ok(());
        }

        let mut children = self.child_entries(parent);
        if !include_trashed {
            children.retain(|entry| !entry.is_trashed());
        }
        sort_entries(&mut children, sort);

        for entry in children {
            let include_entry =
                (!documents_only || entry.is_document()) && (!folders_only || entry.is_folder());
            if include_entry {
                result.push((current_depth, entry));
            }
            if entry.is_folder() {
                if !ancestors.insert(entry.uuid) {
                    return Err(anyhow!(
                        "cycle detected while traversing folder UUID {}",
                        entry.uuid
                    ));
                }
                self.collect_recursive(
                    &Parent::Folder(entry.uuid),
                    current_depth + 1,
                    max_depth,
                    include_trashed,
                    documents_only,
                    folders_only,
                    sort,
                    ancestors,
                    result,
                )?;
                ancestors.remove(&entry.uuid);
            }
        }

        Ok(())
    }
}

pub fn sort_entries(entries: &mut [&DocumentEntry], sort: Option<&SortField>) {
    match sort {
        Some(SortField::Name) | None => {
            entries.sort_by(|a, b| {
                a.is_document().cmp(&b.is_document()).then_with(|| {
                    a.visible_name
                        .to_lowercase()
                        .cmp(&b.visible_name.to_lowercase())
                })
            });
        }
        Some(SortField::Modified) => {
            entries.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
        }
        Some(SortField::Type) => {
            entries.sort_by(|a, b| {
                a.type_sort_key().cmp(&b.type_sort_key()).then_with(|| {
                    a.visible_name
                        .to_lowercase()
                        .cmp(&b.visible_name.to_lowercase())
                })
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{FileType, ItemType};
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
            page_count: None,
        }
    }

    fn make_entry_with_time(
        uuid: &str,
        name: &str,
        item_type: ItemType,
        parent: Parent,
        file_type: Option<FileType>,
        modified_ms: i64,
    ) -> DocumentEntry {
        DocumentEntry {
            last_modified: Utc.timestamp_millis_opt(modified_ms).unwrap(),
            ..make_entry(uuid, name, item_type, parent, file_type)
        }
    }

    // Fixed UUIDs for tests
    const FOLDER_A: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const FOLDER_B: &str = "aaaaaaaa-0000-0000-0000-000000000002";
    const DOC_1: &str = "bbbbbbbb-0000-0000-0000-000000000001";
    const DOC_2: &str = "bbbbbbbb-0000-0000-0000-000000000002";
    const DOC_3: &str = "bbbbbbbb-0000-0000-0000-000000000003";
    const DOC_4: &str = "bbbbbbbb-0000-0000-0000-000000000004";
    const DOC_5: &str = "bbbbbbbb-0000-0000-0000-000000000005";
    const DOC_TRASH: &str = "cccccccc-0000-0000-0000-000000000001";

    fn sample_entries() -> Vec<DocumentEntry> {
        let folder_a_uuid = Uuid::parse_str(FOLDER_A).unwrap();
        vec![
            make_entry(FOLDER_A, "Work", ItemType::Collection, Parent::Root, None),
            make_entry(
                FOLDER_B,
                "Personal",
                ItemType::Collection,
                Parent::Root,
                None,
            ),
            make_entry_with_time(
                DOC_1,
                "Meeting Notes",
                ItemType::Document,
                Parent::Folder(folder_a_uuid),
                Some(FileType::Notebook),
                1710604800000,
            ),
            make_entry_with_time(
                DOC_2,
                "Report.pdf",
                ItemType::Document,
                Parent::Folder(folder_a_uuid),
                Some(FileType::Pdf),
                1710500000000,
            ),
            make_entry(
                DOC_3,
                "Quick Note",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Notebook),
            ),
            make_entry(
                DOC_4,
                "Rust Book",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Epub),
            ),
            make_entry(
                DOC_TRASH,
                "Old Draft",
                ItemType::Document,
                Parent::Trash,
                Some(FileType::Pdf),
            ),
        ]
    }

    fn nested_document_entries() -> Vec<DocumentEntry> {
        let folder_a_uuid = Uuid::parse_str(FOLDER_A).unwrap();
        let folder_b_uuid = Uuid::parse_str(FOLDER_B).unwrap();
        vec![
            make_entry(FOLDER_A, "Work", ItemType::Collection, Parent::Root, None),
            make_entry(
                FOLDER_B,
                "Projects",
                ItemType::Collection,
                Parent::Folder(folder_a_uuid),
                None,
            ),
            make_entry(
                DOC_1,
                "Meeting Notes",
                ItemType::Document,
                Parent::Folder(folder_a_uuid),
                Some(FileType::Notebook),
            ),
            make_entry(
                DOC_5,
                "Design Doc",
                ItemType::Document,
                Parent::Folder(folder_b_uuid),
                Some(FileType::Pdf),
            ),
            make_entry(
                DOC_3,
                "Quick Note",
                ItemType::Document,
                Parent::Root,
                Some(FileType::Notebook),
            ),
        ]
    }

    #[test]
    fn build_and_get() {
        let tree = DocumentTree::build(sample_entries());
        let folder = tree.get(&Uuid::parse_str(FOLDER_A).unwrap()).unwrap();
        assert_eq!(folder.visible_name, "Work");
        assert!(tree.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn root_children() {
        let tree = DocumentTree::build(sample_entries());
        let root = tree.child_entries(&Parent::Root);
        let names: Vec<_> = root.iter().map(|e| e.visible_name.as_str()).collect();
        assert!(names.contains(&"Work"));
        assert!(names.contains(&"Personal"));
        assert!(names.contains(&"Quick Note"));
        assert!(names.contains(&"Rust Book"));
        assert_eq!(root.len(), 4);
    }

    #[test]
    fn folder_children() {
        let tree = DocumentTree::build(sample_entries());
        let folder_uuid = Uuid::parse_str(FOLDER_A).unwrap();
        let children = tree.child_entries(&Parent::Folder(folder_uuid));
        let names: Vec<_> = children.iter().map(|e| e.visible_name.as_str()).collect();
        assert!(names.contains(&"Meeting Notes"));
        assert!(names.contains(&"Report.pdf"));
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn trash_children() {
        let tree = DocumentTree::build(sample_entries());
        let trashed = tree.child_entries(&Parent::Trash);
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].visible_name, "Old Draft");
    }

    #[test]
    fn children_count() {
        let tree = DocumentTree::build(sample_entries());
        assert_eq!(tree.children_count(&Parent::Root), 4);
        assert_eq!(
            tree.children_count(&Parent::Folder(Uuid::parse_str(FOLDER_A).unwrap())),
            2
        );
        assert_eq!(tree.children_count(&Parent::Trash), 1);
        // Non-existent folder
        assert_eq!(tree.children_count(&Parent::Folder(Uuid::new_v4())), 0);
    }

    #[test]
    fn list_children_documents_only() {
        let tree = DocumentTree::build(sample_entries());
        let docs = tree.list_children(&Parent::Root, false, true, false, None);
        assert!(docs.iter().all(|e| e.is_document()));
        assert_eq!(docs.len(), 2); // Quick Note, Rust Book
    }

    #[test]
    fn list_children_folders_only() {
        let tree = DocumentTree::build(sample_entries());
        let folders = tree.list_children(&Parent::Root, false, false, true, None);
        assert!(folders.iter().all(|e| e.is_folder()));
        assert_eq!(folders.len(), 2); // Work, Personal
    }

    #[test]
    fn list_children_default_sort() {
        let tree = DocumentTree::build(sample_entries());
        let items = tree.list_children(&Parent::Root, false, false, false, None);
        let names: Vec<_> = items.iter().map(|e| e.visible_name.as_str()).collect();
        // Default sort: folders first (alpha), then docs (alpha)
        assert_eq!(names, vec!["Personal", "Work", "Quick Note", "Rust Book"]);
    }

    #[test]
    fn list_children_sort_by_modified() {
        let tree = DocumentTree::build(sample_entries());
        let folder_uuid = Uuid::parse_str(FOLDER_A).unwrap();
        let items = tree.list_children(
            &Parent::Folder(folder_uuid),
            false,
            false,
            false,
            Some(&SortField::Modified),
        );
        let names: Vec<_> = items.iter().map(|e| e.visible_name.as_str()).collect();
        // Meeting Notes (1710604800000) > Report.pdf (1710500000000)
        assert_eq!(names, vec!["Meeting Notes", "Report.pdf"]);
    }

    #[test]
    fn list_children_sort_by_type() {
        let tree = DocumentTree::build(sample_entries());
        let items = tree.list_children(&Parent::Root, false, false, false, Some(&SortField::Type));
        let names: Vec<_> = items.iter().map(|e| e.visible_name.as_str()).collect();
        // folders (Personal, Work) → notebook (Quick Note) → epub (Rust Book)
        assert_eq!(names, vec!["Personal", "Work", "Quick Note", "Rust Book"]);
    }

    #[test]
    fn list_recursive_unlimited() {
        let tree = DocumentTree::build(sample_entries());
        let items = tree
            .list_recursive(&Parent::Root, None, false, false, false, None)
            .unwrap();
        // Root items + children of Work and Personal
        // Work (0), Meeting Notes (1), Report.pdf (1), Personal (0), Quick Note (0), Rust Book (0)
        assert_eq!(items.len(), 6);

        // Check depth levels
        let folder_items: Vec<_> = items.iter().filter(|(d, _)| *d == 0).collect();
        assert_eq!(folder_items.len(), 4); // 2 folders + 2 root docs

        let nested_items: Vec<_> = items.iter().filter(|(d, _)| *d == 1).collect();
        assert_eq!(nested_items.len(), 2); // 2 docs in Work
    }

    #[test]
    fn list_recursive_depth_1() {
        let tree = DocumentTree::build(sample_entries());
        let items = tree
            .list_recursive(&Parent::Root, Some(1), false, false, false, None)
            .unwrap();
        // Only root-level items, no descent into folders
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|(d, _)| *d == 0));
    }

    #[test]
    fn list_recursive_depth_2() {
        let tree = DocumentTree::build(sample_entries());
        let items = tree
            .list_recursive(&Parent::Root, Some(2), false, false, false, None)
            .unwrap();
        // Root-level + one level deep
        assert_eq!(items.len(), 6);
    }

    #[test]
    fn list_recursive_documents_only_includes_nested_documents() {
        let tree = DocumentTree::build(nested_document_entries());
        let items = tree
            .list_recursive(&Parent::Root, None, false, true, false, None)
            .unwrap();

        let path_order: Vec<_> = items
            .iter()
            .map(|(depth, entry)| (*depth, entry.visible_name.as_str()))
            .collect();
        assert_eq!(
            path_order,
            vec![(2, "Design Doc"), (1, "Meeting Notes"), (0, "Quick Note")]
        );
        assert!(items.iter().all(|(_, e)| e.is_document()));
    }

    #[test]
    fn list_recursive_errors_on_self_cycle() {
        let folder_uuid = Uuid::parse_str(FOLDER_A).unwrap();
        let tree = DocumentTree::build(vec![make_entry(
            FOLDER_A,
            "Loop",
            ItemType::Collection,
            Parent::Folder(folder_uuid),
            None,
        )]);

        let err = tree
            .list_recursive(
                &Parent::Folder(folder_uuid),
                None,
                false,
                false,
                false,
                None,
            )
            .unwrap_err();

        assert!(err.to_string().contains("cycle detected"));
    }

    #[test]
    fn list_recursive_errors_on_two_folder_cycle() {
        let folder_a_uuid = Uuid::parse_str(FOLDER_A).unwrap();
        let folder_b_uuid = Uuid::parse_str(FOLDER_B).unwrap();
        let tree = DocumentTree::build(vec![
            make_entry(
                FOLDER_A,
                "Folder A",
                ItemType::Collection,
                Parent::Folder(folder_b_uuid),
                None,
            ),
            make_entry(
                FOLDER_B,
                "Folder B",
                ItemType::Collection,
                Parent::Folder(folder_a_uuid),
                None,
            ),
        ]);

        let err = tree
            .list_recursive(
                &Parent::Folder(folder_a_uuid),
                None,
                false,
                false,
                false,
                None,
            )
            .unwrap_err();

        assert!(err.to_string().contains("cycle detected"));
    }

    #[test]
    fn list_trashed_excluded_by_default() {
        let tree = DocumentTree::build(sample_entries());
        let all = tree.list_children(&Parent::Trash, false, false, false, None);
        // Trashed items are excluded when include_trashed=false, but we're
        // listing the Trash parent directly — children of Trash are inherently
        // trashed, so they get filtered out.
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn list_trashed_included() {
        let tree = DocumentTree::build(sample_entries());
        let all = tree.list_children(&Parent::Trash, true, false, false, None);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].visible_name, "Old Draft");
    }

    #[test]
    fn empty_tree() {
        let tree = DocumentTree::build(vec![]);
        assert_eq!(tree.children_count(&Parent::Root), 0);
        assert!(tree.child_entries(&Parent::Root).is_empty());
        assert_eq!(tree.all_entries().count(), 0);
    }

    #[test]
    fn orphan_entry_findable_by_uuid() {
        let orphan_parent = Uuid::parse_str("99999999-9999-9999-9999-999999999999").unwrap();
        let entries = vec![make_entry(
            DOC_1,
            "Orphan",
            ItemType::Document,
            Parent::Folder(orphan_parent),
            Some(FileType::Pdf),
        )];
        let tree = DocumentTree::build(entries);

        // Findable by UUID
        assert!(tree.get(&Uuid::parse_str(DOC_1).unwrap()).is_some());
        // Not in root
        assert!(tree.child_entries(&Parent::Root).is_empty());
        // In its orphan parent bucket
        assert_eq!(tree.children_count(&Parent::Folder(orphan_parent)), 1);
    }
}
