use chrono::{TimeZone, Utc};
use remarkable_cli::cli::{FindArgs, FindTypeFilter, InfoArgs, LsArgs};
use remarkable_cli::commands::{find, info, ls};
use remarkable_cli::error::CliError;
use remarkable_metadata::SortField;
use remarkable_metadata::metadata::{DocumentEntry, FileType, ItemKind, ItemType, Parent};
use remarkable_metadata::tree::{DocumentTree, EntryKindFilter};
use remarkable_tablet::connection::FakeConnection;
use remarkable_tablet::tablet::load_all_metadata;
use uuid::Uuid;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const FOLDER_WORK: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const FOLDER_PROJECTS: &str = "aaaaaaaa-2222-2222-2222-222222222222";
const DOC_NOTES: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const DOC_PAPER: &str = "bbbbbbbb-2222-2222-2222-222222222222";
const DOC_ROOT_EPUB: &str = "bbbbbbbb-3333-3333-3333-333333333333";
const DOC_TRASHED: &str = "cccccccc-1111-1111-1111-111111111111";

fn setup_fake_tablet() -> FakeConnection {
    let conn = FakeConnection::new();
    conn.mkdir(DATA_DIR);

    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_WORK}.metadata"),
        r#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );

    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_PROJECTS}.metadata"),
        format!(r#"{{"visibleName":"Projects","type":"CollectionType","parent":"{FOLDER_WORK}","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}}"#),
    );

    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES}.metadata"),
        format!(r#"{{"visibleName":"Meeting Notes","type":"DocumentType","parent":"{FOLDER_WORK}","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1,"tags":["work","meetings"]}}"#),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES}.content"),
        r#"{"fileType":"notebook","pageCount":12}"#,
    );

    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.metadata"),
        format!(r#"{{"visibleName":"Research Paper","type":"DocumentType","parent":"{FOLDER_PROJECTS}","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1,"tags":["research"]}}"#),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.content"),
        r#"{"fileType":"pdf","pageCount":24}"#,
    );

    conn.set_file(
        &format!("{DATA_DIR}/{DOC_ROOT_EPUB}.metadata"),
        r#"{"visibleName":"Quick Read","type":"DocumentType","parent":"","deleted":false,"pinned":true,"lastModified":1710400000000,"metadatamodified":1710400000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_ROOT_EPUB}.content"),
        r#"{"fileType":"epub","pageCount":300}"#,
    );

    conn.set_file(
        &format!("{DATA_DIR}/{DOC_TRASHED}.metadata"),
        r#"{"visibleName":"Old Draft","type":"DocumentType","parent":"trash","deleted":true,"pinned":false,"lastModified":1710000000000,"metadatamodified":1710000000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_TRASHED}.content"),
        r#"{"fileType":"pdf","pageCount":2}"#,
    );

    conn
}

async fn build_tree(conn: &FakeConnection) -> DocumentTree {
    let entries = load_all_metadata(conn, DATA_DIR).await.unwrap();
    DocumentTree::build(entries)
}

fn ls_args() -> LsArgs {
    LsArgs {
        path_or_uuid: None,
        recursive: false,
        depth: None,
        include_trashed: false,
        sort: None,
        tree: false,
        kind: EntryKindFilter::All,
    }
}

fn find_args(pattern: &str) -> FindArgs {
    FindArgs {
        pattern: pattern.to_string(),
        item_type: None,
        case_sensitive: false,
    }
}

fn flat(out: ls::LsOutput) -> Vec<ls::LsItem> {
    match out {
        ls::LsOutput::Flat(v) => v,
        ls::LsOutput::Tree(_) => panic!("expected flat output"),
    }
}

fn tree_node(out: ls::LsOutput) -> ls::TreeNode {
    match out {
        ls::LsOutput::Tree(n) => n,
        ls::LsOutput::Flat(_) => panic!("expected tree output"),
    }
}

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

// ---------------------------------------------------------------------------
// ls
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ls_root_lists_direct_children_folders_first() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = flat(ls::run_with_tree(&tree, &ls_args()).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Work", "Quick Read"]);
}

#[tokio::test]
async fn ls_path_lists_subfolder_children() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Work".into());
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Projects", "Meeting Notes"]);
}

#[tokio::test]
async fn ls_resolves_uuid_arg() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some(FOLDER_WORK.into());
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Projects", "Meeting Notes"]);
}

#[tokio::test]
async fn ls_recursive_flat_includes_depth() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.recursive = true;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    // Default sort: folders first then alpha. Recursive walks Work, then Projects, then Research Paper, then Meeting Notes, then Quick Read.
    let pairs: Vec<_> = items
        .iter()
        .map(|i| (i.depth.unwrap(), i.entry.name.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (0, "Work"),
            (1, "Projects"),
            (2, "Research Paper"),
            (1, "Meeting Notes"),
            (0, "Quick Read"),
        ]
    );
}

#[tokio::test]
async fn ls_depth_limit() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.depth = Some(1);
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let depths: Vec<_> = items.iter().map(|i| i.depth.unwrap()).collect();
    assert!(depths.iter().all(|d| *d == 0));
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Work", "Quick Read"]);
}

#[tokio::test]
async fn ls_documents_only() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.kind = EntryKindFilter::Documents;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Quick Read"]);
}

#[tokio::test]
async fn ls_folders_only() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.kind = EntryKindFilter::Folders;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Work"]);
}

#[tokio::test]
async fn ls_include_trashed_recursive_surfaces_old_draft() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.recursive = true;
    args.include_trashed = true;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    assert!(
        items
            .iter()
            .any(|i| i.entry.name == "Old Draft" && i.entry.deleted)
    );
}

#[tokio::test]
async fn ls_include_trashed_recursive_name_sort_merges_trash_into_root_order() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.recursive = true;
    args.include_trashed = true;
    args.sort = Some(SortField::Name);
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let pairs: Vec<_> = items
        .iter()
        .map(|i| (i.depth.unwrap(), i.entry.name.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (0, "Work"),
            (1, "Projects"),
            (2, "Research Paper"),
            (1, "Meeting Notes"),
            (0, "Old Draft"),
            (0, "Quick Read"),
        ]
    );
}

#[tokio::test]
async fn ls_include_trashed_flat_root_surfaces_old_draft_under_trash_path() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.include_trashed = true;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    assert_eq!(names, vec!["Work", "Old Draft", "Quick Read"]);
    let trashed = items.iter().find(|i| i.entry.name == "Old Draft").unwrap();
    assert!(trashed.entry.deleted);
    assert_eq!(trashed.entry.path, "/trash/Old Draft");
}

#[tokio::test]
async fn ls_sort_modified_descending() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Work".into());
    args.sort = Some(SortField::Modified);
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.entry.name.as_str()).collect();
    // Meeting Notes (1710604800000) is newer than Projects (1710518400000)
    assert_eq!(names, vec!["Meeting Notes", "Projects"]);
}

#[tokio::test]
async fn ls_on_document_is_invalid_path() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Quick Read".into());
    let err = ls::run_with_tree(&tree, &args).unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::InvalidPath(_)));
}

#[tokio::test]
async fn ls_unknown_path_is_not_found() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Nonexistent".into());
    let err = ls::run_with_tree(&tree, &args).unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::NotFound(_)));
}

#[tokio::test]
async fn ls_populates_page_count_and_children_count() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = flat(ls::run_with_tree(&tree, &ls_args()).unwrap());
    let work = items.iter().find(|i| i.entry.name == "Work").unwrap();
    assert!(matches!(work.entry.kind, ItemKind::Folder));
    assert_eq!(work.children_count, Some(2));

    let quick = items.iter().find(|i| i.entry.name == "Quick Read").unwrap();
    assert!(matches!(
        quick.entry.kind,
        ItemKind::Document {
            file_type: FileType::Epub,
            page_count: Some(300),
        }
    ));
    assert!(quick.entry.pinned);
}

#[tokio::test]
async fn ls_tree_mode_builds_nested_structure() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.tree = true;
    let root = tree_node(ls::run_with_tree(&tree, &args).unwrap());
    assert_eq!(root.name, "/");
    assert!(root.uuid.is_none());
    let top: Vec<_> = root.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(top, vec!["Work", "Quick Read"]);
    let work = root.children.iter().find(|c| c.name == "Work").unwrap();
    let work_kids: Vec<_> = work.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(work_kids, vec!["Projects", "Meeting Notes"]);
    let projects = work.children.iter().find(|c| c.name == "Projects").unwrap();
    let proj_kids: Vec<_> = projects.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(proj_kids, vec!["Research Paper"]);
}

#[tokio::test]
async fn ls_tree_mode_include_trashed_nests_under_virtual_trash() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.tree = true;
    args.include_trashed = true;
    let root = tree_node(ls::run_with_tree(&tree, &args).unwrap());
    let top: Vec<_> = root.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(top, vec!["Work", "Quick Read", "trash"]);
    let trash = root.children.iter().find(|c| c.name == "trash").unwrap();
    assert!(trash.uuid.is_none());
    let trash_kids: Vec<_> = trash.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(trash_kids, vec!["Old Draft"]);
}

#[tokio::test]
async fn ls_tree_carries_page_count_for_documents_only() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.tree = true;
    let root = tree_node(ls::run_with_tree(&tree, &args).unwrap());

    // Folders: kind is Folder, no page_count field at all.
    let work = root.children.iter().find(|c| c.name == "Work").unwrap();
    assert!(matches!(work.kind, ItemKind::Folder));

    // Documents: page_count populated from .content.
    let quick = root
        .children
        .iter()
        .find(|c| c.name == "Quick Read")
        .unwrap();
    assert!(matches!(
        quick.kind,
        ItemKind::Document {
            page_count: Some(300),
            ..
        }
    ));

    let meeting = work
        .children
        .iter()
        .find(|c| c.name == "Meeting Notes")
        .unwrap();
    assert!(matches!(
        meeting.kind,
        ItemKind::Document {
            page_count: Some(12),
            ..
        }
    ));
}

#[tokio::test]
async fn load_diagnostics_records_timing() {
    use remarkable_tablet::tablet::load_all_metadata_full;
    let conn = setup_fake_tablet();
    let (_entries, diag) = load_all_metadata_full(&conn, DATA_DIR).await.unwrap();
    // FakeConnection serves files from a tempdir, so timing is non-zero but small.
    assert!(diag.list_dir_elapsed.as_nanos() > 0);
    assert!(diag.read_elapsed.as_nanos() > 0);
    assert!(diag.dir_entry_count > 0);
    assert!(diag.uuid_metadata_count > 0);
}

#[tokio::test]
async fn ls_tree_subfolder_uses_entry_as_root() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.tree = true;
    args.path_or_uuid = Some("/Work".into());
    let root = tree_node(ls::run_with_tree(&tree, &args).unwrap());
    assert_eq!(root.name, "Work");
    assert_eq!(root.uuid, Some(Uuid::parse_str(FOLDER_WORK).unwrap()));
}

#[tokio::test]
async fn ls_tree_mode_errors_on_folder_cycle() {
    let folder_uuid = Uuid::parse_str(FOLDER_WORK).unwrap();
    let tree = DocumentTree::build(vec![make_entry(
        FOLDER_WORK,
        "Loop",
        ItemType::Collection,
        Parent::Folder(folder_uuid),
        None,
    )]);
    let mut args = ls_args();
    args.tree = true;
    args.path_or_uuid = Some(FOLDER_WORK.into());
    let err = ls::run_with_tree(&tree, &args).unwrap_err();
    assert!(err.to_string().contains("cycle detected"));
}

// ---------------------------------------------------------------------------
// info
// ---------------------------------------------------------------------------

#[tokio::test]
async fn info_returns_metadata_and_content() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let args = InfoArgs {
        path_or_uuid: "/Work/Meeting Notes".into(),
    };
    let out = info::run_with_conn(&conn, DATA_DIR, &tree, &args)
        .await
        .unwrap();
    assert_eq!(out.entry.uuid, Uuid::parse_str(DOC_NOTES).unwrap());
    assert_eq!(out.entry.path, "/Work/Meeting Notes");
    assert_eq!(out.entry.name, "Meeting Notes");
    assert!(matches!(
        out.entry.kind,
        ItemKind::Document {
            file_type: FileType::Notebook,
            page_count: Some(12),
        }
    ));
    assert_eq!(out.entry.tags, vec!["work", "meetings"]);
    let content = out.content.as_ref().unwrap();
    assert_eq!(content["fileType"], "notebook");
    assert_eq!(content["pageCount"], 12);
    assert_eq!(out.metadata["visibleName"], "Meeting Notes");
}

#[tokio::test]
async fn info_uuid_matches_path() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let by_path = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/Work/Meeting Notes".into(),
        },
    )
    .await
    .unwrap();
    let by_uuid = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: DOC_NOTES.into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(by_path.entry.uuid, by_uuid.entry.uuid);
    assert_eq!(by_path.entry.path, by_uuid.entry.path);
}

#[tokio::test]
async fn info_orphan_uuid_uses_fallback_path() {
    let conn = FakeConnection::new();
    conn.mkdir(DATA_DIR);
    let orphan_parent = "dddddddd-1111-1111-1111-111111111111";
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.metadata"),
        format!(
            r#"{{"visibleName":"Orphaned","type":"DocumentType","parent":"{orphan_parent}","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1}}"#
        ),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.content"),
        r#"{"fileType":"pdf"}"#,
    );
    let tree = build_tree(&conn).await;
    let out = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: DOC_PAPER.into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(out.entry.uuid, Uuid::parse_str(DOC_PAPER).unwrap());
    assert_eq!(out.entry.path, "/Orphaned");
}

#[tokio::test]
async fn info_uuid_in_parent_cycle_uses_fallback_path() {
    let conn = FakeConnection::new();
    conn.mkdir(DATA_DIR);
    let folder_a = "dddddddd-1111-1111-1111-111111111111";
    let folder_b = "dddddddd-2222-2222-2222-222222222222";
    let doc_uuid = "eeeeeeee-1111-1111-1111-111111111111";

    conn.set_file(
        &format!("{DATA_DIR}/{folder_a}.metadata"),
        format!(
            r#"{{"visibleName":"Folder A","type":"CollectionType","parent":"{folder_b}","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}}"#
        ),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{folder_b}.metadata"),
        format!(
            r#"{{"visibleName":"Folder B","type":"CollectionType","parent":"{folder_a}","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}}"#
        ),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{doc_uuid}.metadata"),
        format!(
            r#"{{"visibleName":"Looped Doc","type":"DocumentType","parent":"{folder_b}","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1}}"#
        ),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{doc_uuid}.content"),
        r#"{"fileType":"pdf"}"#,
    );

    let tree = build_tree(&conn).await;
    let out = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: doc_uuid.into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(out.entry.uuid, Uuid::parse_str(doc_uuid).unwrap());
    assert_eq!(out.entry.path, "/Looped Doc");
}

#[tokio::test]
async fn info_root_is_invalid_path() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let err = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/".into(),
        },
    )
    .await
    .unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::InvalidPath(_)));
}

#[tokio::test]
async fn info_missing_target_is_not_found() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let err = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/Nonexistent".into(),
        },
    )
    .await
    .unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::NotFound(_)));
}

#[tokio::test]
async fn info_document_without_content_is_dropped_from_tree() {
    // A document whose `.content` is missing on disk is not classifiable; the
    // loader drops it and records the failure in diagnostics. `info` sees an
    // empty tree and returns NotFound.
    use remarkable_tablet::tablet::load_all_metadata_full;
    let conn = FakeConnection::new();
    conn.mkdir(DATA_DIR);
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER}.metadata"),
        r#"{"visibleName":"Research Paper","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1,"tags":["research"]}"#,
    );

    let (entries, diag) = load_all_metadata_full(&conn, DATA_DIR).await.unwrap();
    assert!(entries.is_empty());
    assert_eq!(diag.content_failures.len(), 1);
    assert_eq!(
        diag.content_failures[0].0,
        Uuid::parse_str(DOC_PAPER).unwrap()
    );

    let tree = DocumentTree::build(entries);
    let err = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/Research Paper".into(),
        },
    )
    .await
    .unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::NotFound(_)));
}

#[tokio::test]
async fn info_document_content_read_failure_drops_from_tree() {
    // A document whose `.content` exists but cannot be read at load time is
    // treated identically to the missing case: dropped, surfaced via
    // diagnostics. `info` then returns NotFound.
    use remarkable_tablet::tablet::load_all_metadata_full;
    let conn = setup_fake_tablet();
    let path = format!("{DATA_DIR}/{DOC_PAPER}.content");
    conn.set_read_error(&path, "permission denied");

    let (entries, diag) = load_all_metadata_full(&conn, DATA_DIR).await.unwrap();
    let paper_uuid = Uuid::parse_str(DOC_PAPER).unwrap();
    assert!(entries.iter().all(|e| e.uuid != paper_uuid));
    assert!(
        diag.content_failures
            .iter()
            .any(|(uuid, _)| *uuid == paper_uuid)
    );

    let tree = DocumentTree::build(entries);
    let err = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/Work/Projects/Research Paper".into(),
        },
    )
    .await
    .unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::NotFound(_)));
}

#[tokio::test]
async fn info_folder_returns_no_content() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let out = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/Work".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(out.entry.kind, ItemKind::Folder));
    assert!(out.content.is_none());
}

// ---------------------------------------------------------------------------
// find
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_substring_case_insensitive_default() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(&tree, &find_args("meeting")).unwrap();
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Meeting Notes"]);
}

#[tokio::test]
async fn find_substring_case_sensitive() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = find_args("meeting");
    args.case_sensitive = true;
    let items = find::run_with_tree(&tree, &args).unwrap();
    assert!(items.is_empty());

    let items = find::run_with_tree(
        &tree,
        &FindArgs {
            pattern: "Meeting".into(),
            item_type: None,
            case_sensitive: true,
        },
    )
    .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn find_filters_by_type() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let folders = find::run_with_tree(
        &tree,
        &FindArgs {
            pattern: String::new(),
            item_type: Some(FindTypeFilter::Folder),
            case_sensitive: false,
        },
    )
    .unwrap();
    // Sorted by path: "/Work" comes before "/Work/Projects".
    let names: Vec<_> = folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["Work", "Projects"]);

    let docs = find::run_with_tree(
        &tree,
        &FindArgs {
            pattern: String::new(),
            item_type: Some(FindTypeFilter::Document),
            case_sensitive: false,
        },
    )
    .unwrap();
    // Sorted by path: "/Quick Read" < "/Work/Meeting Notes" < "/Work/Projects/Research Paper".
    let names: Vec<_> = docs.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["Quick Read", "Meeting Notes", "Research Paper"]);
}

#[tokio::test]
async fn find_returns_empty_when_no_matches() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(&tree, &find_args("xyz-does-not-exist")).unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn find_excludes_trashed_by_default() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(&tree, &find_args("Old Draft")).unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn find_glob_star_matches() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(&tree, &find_args("Meet*")).unwrap();
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Meeting Notes"]);
}

#[tokio::test]
async fn find_glob_question_mark_matches_single_char() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(&tree, &find_args("?eeting Notes")).unwrap();
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Meeting Notes"]);
}

#[tokio::test]
async fn find_glob_no_matches() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(&tree, &find_args("Z*")).unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn find_glob_case_sensitive_does_not_match_lowercase() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = find::run_with_tree(
        &tree,
        &FindArgs {
            pattern: "meet*".into(),
            item_type: None,
            case_sensitive: true,
        },
    )
    .unwrap();
    assert!(items.is_empty());

    let items = find::run_with_tree(
        &tree,
        &FindArgs {
            pattern: "meet*".into(),
            item_type: None,
            case_sensitive: false,
        },
    )
    .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn find_invalid_glob_returns_error() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let err = find::run_with_tree(&tree, &find_args("[unclosed*")).unwrap_err();
    let cli = remarkable_cli::commands::common::to_cli_error(err);
    assert!(matches!(cli, CliError::InvalidPath(_)));
}
