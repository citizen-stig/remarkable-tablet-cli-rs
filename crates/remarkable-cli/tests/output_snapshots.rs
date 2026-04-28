//! Snapshot tests for the human and JSON output of each subcommand.
//!
//! These exercise the same library entry points the binary uses
//! (`run_with_tree` / `run_with_conn` / `fetch_device_info`) and snapshot
//! the formatted output. The fake tablet fixture is duplicated from
//! `tests/browse.rs` deliberately — keeping the two suites independent
//! makes it obvious when a snapshot drift is intended.

use insta::assert_snapshot;
use remarkable_cli::cli::{FindArgs, InfoArgs, LsArgs};
use remarkable_cli::commands::{find, info, ls};
use remarkable_tablet::connection::FakeConnection;
use remarkable_cli::output::{self, OutputFormat};
use remarkable_tablet::tablet::{self, load_all_metadata};
use remarkable_metadata::tree::{DocumentTree, EntryKindFilter};

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

// ---------------------------------------------------------------------------
// ls
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ls_root_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let out = ls::run_with_tree(&tree, &ls_args()).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn ls_root_json() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let out = ls::run_with_tree(&tree, &ls_args()).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Json));
}

#[tokio::test]
async fn ls_subfolder_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = LsArgs {
        path_or_uuid: Some("/Work".to_string()),
        ..ls_args()
    };
    let out = ls::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn ls_recursive_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = LsArgs {
        recursive: true,
        ..ls_args()
    };
    let out = ls::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn ls_tree_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = LsArgs {
        tree: true,
        ..ls_args()
    };
    let out = ls::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn ls_tree_json() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = LsArgs {
        tree: true,
        ..ls_args()
    };
    let out = ls::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Json));
}

#[tokio::test]
async fn ls_include_trashed_recursive_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = LsArgs {
        recursive: true,
        include_trashed: true,
        ..ls_args()
    };
    let out = ls::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn ls_documents_only_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = LsArgs {
        recursive: true,
        kind: EntryKindFilter::Documents,
        ..ls_args()
    };
    let out = ls::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(ls::format_output(&out, OutputFormat::Human));
}

// ---------------------------------------------------------------------------
// info
// ---------------------------------------------------------------------------

#[tokio::test]
async fn info_document_human() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let args = InfoArgs {
        path_or_uuid: "/Work/Meeting Notes".to_string(),
    };
    let out = info::run_with_conn(&conn, DATA_DIR, &tree, &args)
        .await
        .unwrap();
    assert_snapshot!(info::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn info_document_json() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let args = InfoArgs {
        path_or_uuid: "/Work/Meeting Notes".to_string(),
    };
    let out = info::run_with_conn(&conn, DATA_DIR, &tree, &args)
        .await
        .unwrap();
    assert_snapshot!(info::format_output(&out, OutputFormat::Json));
}

#[tokio::test]
async fn info_folder_human() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let args = InfoArgs {
        path_or_uuid: "/Work".to_string(),
    };
    let out = info::run_with_conn(&conn, DATA_DIR, &tree, &args)
        .await
        .unwrap();
    assert_snapshot!(info::format_output(&out, OutputFormat::Human));
}

#[tokio::test]
async fn info_pinned_epub_human() {
    let conn = setup_fake_tablet();
    let tree = build_tree(&conn).await;
    let args = InfoArgs {
        path_or_uuid: "/Quick Read".to_string(),
    };
    let out = info::run_with_conn(&conn, DATA_DIR, &tree, &args)
        .await
        .unwrap();
    assert_snapshot!(info::format_output(&out, OutputFormat::Human));
}

// ---------------------------------------------------------------------------
// find
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_substring_match_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = FindArgs {
        pattern: "Notes".to_string(),
        item_type: None,
        case_sensitive: false,
    };
    let items = find::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(find::format_output(&items, OutputFormat::Human));
}

#[tokio::test]
async fn find_substring_match_json() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = FindArgs {
        pattern: "Notes".to_string(),
        item_type: None,
        case_sensitive: false,
    };
    let items = find::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(find::format_output(&items, OutputFormat::Json));
}

#[tokio::test]
async fn find_no_match_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = FindArgs {
        pattern: "ZZZZ".to_string(),
        item_type: None,
        case_sensitive: false,
    };
    let items = find::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(find::format_output(&items, OutputFormat::Human));
}

#[tokio::test]
async fn find_glob_match_human() {
    let tree = build_tree(&setup_fake_tablet()).await;
    let args = FindArgs {
        pattern: "*Paper*".to_string(),
        item_type: None,
        case_sensitive: false,
    };
    let items = find::run_with_tree(&tree, &args).unwrap();
    assert_snapshot!(find::format_output(&items, OutputFormat::Human));
}

// ---------------------------------------------------------------------------
// connect (device info)
// ---------------------------------------------------------------------------

fn setup_device_info_conn() -> FakeConnection {
    let conn = FakeConnection::new();
    conn.set_file("/etc/version", "20230412102300\n");
    conn.mkdir("/sys/class/power_supply/max77818_battery");
    conn.set_file("/sys/class/power_supply/max77818_battery/capacity", "78\n");
    conn.set_command_output(
        "df -k",
        "Filesystem     1K-blocks    Used Available Use% Mounted on\n\
         /dev/root        6291456 2097152   4194304  33% /\n",
    );
    conn
}

#[tokio::test]
async fn connect_human() {
    let conn = setup_device_info_conn();
    let info = tablet::fetch_device_info(&conn, "10.11.99.1", DATA_DIR)
        .await
        .unwrap();
    assert_snapshot!(output::format_device_info(&info, OutputFormat::Human));
}

#[tokio::test]
async fn connect_json() {
    let conn = setup_device_info_conn();
    let info = tablet::fetch_device_info(&conn, "10.11.99.1", DATA_DIR)
        .await
        .unwrap();
    assert_snapshot!(output::format_device_info(&info, OutputFormat::Json));
}
