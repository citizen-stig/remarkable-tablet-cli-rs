use remarkable_tablet_cli_rs::metadata::{self, FileType, ItemType, Parent};
use uuid::Uuid;

#[test]
fn fixture_folder_root() {
    let data = include_bytes!("fixtures/folder_root.metadata");
    let m = metadata::parse_metadata(data).unwrap();
    assert_eq!(m.visible_name, "Work");
    assert_eq!(m.item_type, ItemType::Collection);
    assert_eq!(m.parent, Parent::Root);
    assert!(!m.deleted);
    assert!(!m.pinned);
    assert_eq!(m.last_modified.timestamp_millis(), 1_710_518_400_000);
    assert_eq!(m.version, 1);
    assert!(m.tags.is_empty());
    assert!(m.last_opened.is_none());
}

#[test]
fn fixture_doc_in_folder() {
    let meta = include_bytes!("fixtures/doc_in_folder.metadata");
    let content = include_bytes!("fixtures/doc_in_folder.content");

    let m = metadata::parse_metadata(meta).unwrap();
    assert_eq!(m.visible_name, "Meeting Notes");
    assert_eq!(m.item_type, ItemType::Document);
    assert_eq!(
        m.parent,
        Parent::Folder(Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap())
    );
    assert_eq!(m.tags, vec!["work", "meetings"]);
    assert!(m.last_opened.is_some());

    let c = metadata::parse_content(content).unwrap();
    assert_eq!(c.file_type, FileType::Notebook);
    assert_eq!(c.effective_page_count(), Some(12));
}

#[test]
fn fixture_doc_trashed() {
    let meta = include_bytes!("fixtures/doc_trashed.metadata");
    let content = include_bytes!("fixtures/doc_trashed.content");

    let m = metadata::parse_metadata(meta).unwrap();
    assert_eq!(m.visible_name, "Old Draft");
    assert_eq!(m.parent, Parent::Trash);
    assert!(m.deleted);

    let c = metadata::parse_content(content).unwrap();
    assert_eq!(c.file_type, FileType::Pdf);
    assert_eq!(c.effective_page_count(), Some(3));
}

#[test]
fn fixture_doc_minimal() {
    let data = include_bytes!("fixtures/doc_minimal.metadata");
    let m = metadata::parse_metadata(data).unwrap();
    assert_eq!(m.visible_name, "Quick Note");
    assert_eq!(m.parent, Parent::Root);
    assert!(m.tags.is_empty());
    assert!(m.last_opened.is_none());
}

#[test]
fn fixture_doc_epub() {
    let meta = include_bytes!("fixtures/doc_epub.metadata");
    let content = include_bytes!("fixtures/doc_epub.content");

    let m = metadata::parse_metadata(meta).unwrap();
    assert_eq!(m.visible_name, "Rust Programming");
    assert!(m.pinned);
    assert_eq!(m.tags, vec!["programming"]);

    let c = metadata::parse_content(content).unwrap();
    assert_eq!(c.file_type, FileType::Epub);
    assert_eq!(c.effective_page_count(), Some(412));
}
