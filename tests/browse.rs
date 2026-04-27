use remarkable_tablet_cli_rs::cli::{FindArgs, FindTypeFilter, InfoArgs, LsArgs, SortField};
use remarkable_tablet_cli_rs::commands::{find, info, ls};
use remarkable_tablet_cli_rs::connection::FakeConnection;
use remarkable_tablet_cli_rs::error::CliError;
use remarkable_tablet_cli_rs::metadata::FileType;
use remarkable_tablet_cli_rs::tablet::load_all_metadata;
use remarkable_tablet_cli_rs::tree::DocumentTree;
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
        documents_only: false,
        folders_only: false,
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

// ---------------------------------------------------------------------------
// ls
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ls_root_lists_direct_children_folders_first() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = flat(ls::run_with_tree(&tree, &ls_args()).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Work", "Quick Read"]);
}

#[tokio::test]
async fn ls_path_lists_subfolder_children() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Work".into());
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Projects", "Meeting Notes"]);
}

#[tokio::test]
async fn ls_resolves_uuid_arg() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some(FOLDER_WORK.into());
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
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
        .map(|i| (i.depth.unwrap(), i.name.as_str()))
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
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Work", "Quick Read"]);
}

#[tokio::test]
async fn ls_documents_only() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.documents_only = true;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Quick Read"]);
}

#[tokio::test]
async fn ls_folders_only() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.folders_only = true;
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
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
    assert!(items.iter().any(|i| i.name == "Old Draft" && i.deleted));
}

#[tokio::test]
async fn ls_sort_modified_descending() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Work".into());
    args.sort = Some(SortField::Modified);
    let items = flat(ls::run_with_tree(&tree, &args).unwrap());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
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
    let cli = err.downcast_ref::<CliError>().unwrap();
    assert!(matches!(cli, CliError::InvalidPath(_)));
}

#[tokio::test]
async fn ls_unknown_path_is_not_found() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.path_or_uuid = Some("/Nonexistent".into());
    let err = ls::run_with_tree(&tree, &args).unwrap_err();
    let cli = err.downcast_ref::<CliError>().unwrap();
    assert!(matches!(cli, CliError::NotFound(_)));
}

#[tokio::test]
async fn ls_populates_page_count_and_children_count() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let items = flat(ls::run_with_tree(&tree, &ls_args()).unwrap());
    let work = items.iter().find(|i| i.name == "Work").unwrap();
    assert_eq!(work.kind, ls::ItemKind::Folder);
    assert_eq!(work.children_count, Some(2));
    assert_eq!(work.page_count, None);

    let quick = items.iter().find(|i| i.name == "Quick Read").unwrap();
    assert_eq!(quick.kind, ls::ItemKind::Document);
    assert_eq!(quick.file_type, Some(FileType::Epub));
    assert_eq!(quick.page_count, Some(300));
    assert!(quick.pinned);
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
async fn ls_tree_carries_page_count_for_documents_only() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let mut args = ls_args();
    args.tree = true;
    let root = tree_node(ls::run_with_tree(&tree, &args).unwrap());

    // Folders: page_count is None.
    let work = root.children.iter().find(|c| c.name == "Work").unwrap();
    assert_eq!(work.page_count, None);

    // Documents: page_count populated from .content.
    let quick = root
        .children
        .iter()
        .find(|c| c.name == "Quick Read")
        .unwrap();
    assert_eq!(quick.page_count, Some(300));

    let meeting = work
        .children
        .iter()
        .find(|c| c.name == "Meeting Notes")
        .unwrap();
    assert_eq!(meeting.page_count, Some(12));
}

#[tokio::test]
async fn load_diagnostics_records_timing() {
    use remarkable_tablet_cli_rs::tablet::load_all_metadata_full;
    let conn = setup_fake_tablet();
    let (_entries, diag) = load_all_metadata_full(&conn, DATA_DIR).await.unwrap();
    // FakeConnection serves files from a tempdir, so timing is non-zero but small.
    assert!(diag.list_dir_elapsed.as_nanos() > 0);
    assert!(diag.read_elapsed.as_nanos() > 0);
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
    assert_eq!(out.uuid, Uuid::parse_str(DOC_NOTES).unwrap());
    assert_eq!(out.path, "/Work/Meeting Notes");
    assert_eq!(out.name, "Meeting Notes");
    assert_eq!(out.kind, ls::ItemKind::Document);
    assert_eq!(out.file_type, Some(FileType::Notebook));
    assert_eq!(out.tags, vec!["work", "meetings"]);
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
    assert_eq!(by_path.uuid, by_uuid.uuid);
    assert_eq!(by_path.path, by_uuid.path);
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
    let cli = err.downcast_ref::<CliError>().unwrap();
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
    let cli = err.downcast_ref::<CliError>().unwrap();
    assert!(matches!(cli, CliError::NotFound(_)));
}

#[tokio::test]
async fn info_document_without_content_yields_null_content() {
    let conn = setup_fake_tablet();
    // Wipe the .content file so the read fails.
    let path = format!("{DATA_DIR}/{DOC_PAPER}.content");
    conn.set_read_error(&path, "missing");
    let tree = build_tree(&conn).await;
    let out = info::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &InfoArgs {
            path_or_uuid: "/Work/Projects/Research Paper".into(),
        },
    )
    .await
    .unwrap();
    assert!(out.content.is_none());
    assert_eq!(out.metadata["visibleName"], "Research Paper");
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
    assert_eq!(out.kind, ls::ItemKind::Folder);
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
            pattern: "".into(),
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
            pattern: "".into(),
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
    let cli = err.downcast_ref::<CliError>().unwrap();
    assert!(matches!(cli, CliError::InvalidPath(_)));
}
