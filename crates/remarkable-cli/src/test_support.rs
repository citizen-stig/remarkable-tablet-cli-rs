//! `DocumentEntry` fixture constructors shared by the command modules' unit
//! tests. Compiled only under `#[cfg(test)]`.

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use remarkable_metadata::metadata::{DocumentEntry, FileType, ItemKind, Parent};

/// Fixed `lastModified` stamp so fixtures are deterministic.
fn fixture_timestamp() -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(1_710_000_000_000).unwrap()
}

/// Build a document [`DocumentEntry`] fixture. Trashed iff `parent` is the trash.
pub(crate) fn make_doc(
    uuid: &str,
    name: &str,
    parent: Parent,
    file_type: FileType,
) -> DocumentEntry {
    let deleted = parent == Parent::Trash;
    DocumentEntry {
        uuid: Uuid::parse_str(uuid).unwrap(),
        visible_name: name.to_string(),
        kind: ItemKind::Document {
            file_type,
            page_count: None,
        },
        parent,
        deleted,
        pinned: false,
        last_modified: fixture_timestamp(),
        version: 1,
        tags: vec![],
        last_opened: None,
    }
}

/// Build a folder [`DocumentEntry`] fixture. Trashed iff `parent` is the trash.
pub(crate) fn make_folder(uuid: &str, name: &str, parent: Parent) -> DocumentEntry {
    let deleted = parent == Parent::Trash;
    DocumentEntry {
        uuid: Uuid::parse_str(uuid).unwrap(),
        visible_name: name.to_string(),
        kind: ItemKind::Folder,
        parent,
        deleted,
        pinned: false,
        last_modified: fixture_timestamp(),
        version: 1,
        tags: vec![],
        last_opened: None,
    }
}
