use remarkable_tablet_cli_rs::connection::FakeConnection;
use remarkable_tablet_cli_rs::metadata::{FileType, Parent};
use remarkable_tablet_cli_rs::path_resolver::{self, Resolved};
use remarkable_tablet_cli_rs::tablet::load_all_metadata;
use remarkable_tablet_cli_rs::tree::{DocumentTree, ListFilter};
use uuid::Uuid;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const FOLDER_WORK: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const FOLDER_PROJECTS: &str = "aaaaaaaa-2222-2222-2222-222222222222";
const DOC_NOTES: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const DOC_PAPER: &str = "bbbbbbbb-2222-2222-2222-222222222222";
const DOC_ROOT_PDF: &str = "bbbbbbbb-3333-3333-3333-333333333333";
const DOC_TRASHED: &str = "cccccccc-1111-1111-1111-111111111111";

fn setup_fake_tablet() -> FakeConnection {
    let conn = FakeConnection::new();
    conn.mkdir(DATA_DIR);

    // Root folder: Work
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_WORK}.metadata"),
        r#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );

    // Nested folder: Work/Projects
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_PROJECTS}.metadata"),
        format!(r#"{{"visibleName":"Projects","type":"CollectionType","parent":"{FOLDER_WORK}","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}}"#),
    );

    // Document: Work/Meeting Notes (notebook)
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES}.metadata"),
        format!(r#"{{"visibleName":"Meeting Notes","type":"DocumentType","parent":"{FOLDER_WORK}","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1,"tags":["work","meetings"]}}"#),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES}.content"),
        r#"{"fileType":"notebook"}"#,
    );

    // Document: Work/Projects/Research Paper (pdf)
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.metadata"),
        format!(r#"{{"visibleName":"Research Paper","type":"DocumentType","parent":"{FOLDER_PROJECTS}","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1,"tags":["research"]}}"#),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.content"),
        r#"{"fileType":"pdf"}"#,
    );

    // Root document: Quick Read (epub)
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_ROOT_PDF}.metadata"),
        r#"{"visibleName":"Quick Read","type":"DocumentType","parent":"","deleted":false,"pinned":true,"lastModified":1710400000000,"metadatamodified":1710400000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_ROOT_PDF}.content"),
        r#"{"fileType":"epub"}"#,
    );

    // Trashed document
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_TRASHED}.metadata"),
        r#"{"visibleName":"Old Draft","type":"DocumentType","parent":"trash","deleted":true,"pinned":false,"lastModified":1710000000000,"metadatamodified":1710000000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_TRASHED}.content"),
        r#"{"fileType":"pdf"}"#,
    );

    conn
}

#[tokio::test]
async fn full_pipeline_load_build_resolve() {
    let conn = setup_fake_tablet();
    let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
    assert_eq!(entries.len(), 6);

    let tree = DocumentTree::build(entries);

    // Root has: Work folder + Quick Read doc
    let root = tree.child_entries(&Parent::Root);
    assert_eq!(root.len(), 2);

    // Work folder has: Projects folder + Meeting Notes doc
    let work_children = tree.child_entries(&Parent::Folder(Uuid::parse_str(FOLDER_WORK).unwrap()));
    assert_eq!(work_children.len(), 2);

    // Projects folder has: Research Paper
    let project_children =
        tree.child_entries(&Parent::Folder(Uuid::parse_str(FOLDER_PROJECTS).unwrap()));
    assert_eq!(project_children.len(), 1);
    assert_eq!(project_children[0].visible_name, "Research Paper");
    assert_eq!(project_children[0].file_type(), Some(FileType::Pdf));

    // Trash has: Old Draft
    let trash = tree.list_children(&Parent::Trash, ListFilter::all().include_trashed());
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].visible_name, "Old Draft");
}

#[tokio::test]
async fn path_resolution_end_to_end() {
    let conn = setup_fake_tablet();
    let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
    let tree = DocumentTree::build(entries);

    // Resolve root
    assert!(matches!(
        path_resolver::resolve(&tree, "/"),
        Ok(Resolved::Root)
    ));

    // Resolve by path
    match path_resolver::resolve(&tree, "/Work") {
        Ok(Resolved::Entry(e)) => {
            assert_eq!(e.visible_name, "Work");
            assert!(e.is_folder());
        }
        other => panic!("expected Entry(Work), got {other:?}"),
    }

    // Resolve nested path
    match path_resolver::resolve(&tree, "/Work/Meeting Notes") {
        Ok(Resolved::Entry(e)) => {
            assert_eq!(e.visible_name, "Meeting Notes");
            assert_eq!(e.file_type(), Some(FileType::Notebook));
            assert_eq!(e.tags, vec!["work", "meetings"]);
        }
        other => panic!("expected Entry(Meeting Notes), got {other:?}"),
    }

    // Resolve deeply nested
    match path_resolver::resolve(&tree, "/Work/Projects/Research Paper") {
        Ok(Resolved::Entry(e)) => assert_eq!(e.visible_name, "Research Paper"),
        other => panic!("expected Entry(Research Paper), got {other:?}"),
    }

    // Resolve by UUID
    match path_resolver::resolve(&tree, DOC_NOTES) {
        Ok(Resolved::Entry(e)) => assert_eq!(e.visible_name, "Meeting Notes"),
        other => panic!("expected Entry(Meeting Notes), got {other:?}"),
    }

    // UUID to path
    let uuid = Uuid::parse_str(DOC_PAPER).unwrap();
    let path = path_resolver::resolve_uuid_to_path(&tree, &uuid).unwrap();
    assert_eq!(path, "/Work/Projects/Research Paper");
}

#[tokio::test]
async fn recursive_tree_listing() {
    let conn = setup_fake_tablet();
    let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
    let tree = DocumentTree::build(entries);

    // Full recursive listing from root (excluding trash)
    let all = tree
        .list_recursive(&Parent::Root, None, ListFilter::default())
        .unwrap();
    // Work(0), Meeting Notes(1), Projects(1), Research Paper(2), Quick Read(0)
    assert_eq!(all.len(), 5);

    // Check depth levels
    let depth_0: Vec<_> = all.iter().filter(|(d, _)| *d == 0).collect();
    assert_eq!(depth_0.len(), 2); // Work, Quick Read

    let depth_1: Vec<_> = all.iter().filter(|(d, _)| *d == 1).collect();
    assert_eq!(depth_1.len(), 2); // Meeting Notes, Projects

    let depth_2: Vec<_> = all.iter().filter(|(d, _)| *d == 2).collect();
    assert_eq!(depth_2.len(), 1); // Research Paper
}
