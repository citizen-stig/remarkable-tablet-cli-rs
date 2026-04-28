use std::path::{Path, PathBuf};

use remarkable_cli::cli::UploadArgs;
use remarkable_cli::commands::upload;
use remarkable_tablet::connection::{FakeConnection, TabletConnection};
use remarkable_cli::error::CliError;
use remarkable_metadata::metadata::{FileType, ItemType, Parent, RawContent, RawMetadata};
use remarkable_tablet::tablet::load_all_metadata;
use remarkable_metadata::tree::DocumentTree;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const FOLDER_RESEARCH_UUID: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const FOLDER_TRASHED_UUID: &str = "aaaaaaaa-2222-2222-2222-222222222222";
const DOC_PAPER_UUID: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const DOC_DUP_UUID: &str = "cccccccc-1111-1111-1111-111111111111";

fn populate(conn: &FakeConnection) {
    conn.mkdir(DATA_DIR);

    // Folder "Research" at root — destination for several happy-path tests.
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_RESEARCH_UUID}.metadata"),
        br#"{"visibleName":"Research","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );

    // Trashed folder — used to confirm uploads into trash are rejected.
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_TRASHED_UUID}.metadata"),
        br#"{"visibleName":"Old Folder","type":"CollectionType","parent":"trash","deleted":true,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );

    // Existing PDF at root — used for "parent must be a folder" rejection.
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER_UUID}.metadata"),
        br#"{"visibleName":"Paper Draft","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PAPER_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );

    // Existing document named "duplicate" at root — used to surface a warning
    // when a new upload reuses the same name in the same folder.
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_DUP_UUID}.metadata"),
        br#"{"visibleName":"duplicate","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_DUP_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );
}

async fn build_tree(conn: &FakeConnection) -> DocumentTree {
    let entries = load_all_metadata(conn, DATA_DIR).await.unwrap();
    DocumentTree::build(entries)
}

fn register_xochitl(conn: &FakeConnection) {
    conn.set_command_output("systemctl stop xochitl", "");
    conn.set_command_output("systemctl start xochitl", "");
}

fn args(files: &[&Path], parent: Option<&str>, name: Option<&str>, dry_run: bool) -> UploadArgs {
    UploadArgs {
        files: files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        parent: parent.map(String::from),
        name: name.map(String::from),
        dry_run,
    }
}

fn make_local(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

fn downcast_cli(err: anyhow::Error) -> CliError {
    err.downcast::<CliError>().expect("expected CliError")
}

async fn remote_files(conn: &FakeConnection) -> Vec<String> {
    let mut files = remarkable_tablet::transfer::walk_remote(conn, DATA_DIR)
        .await
        .unwrap();
    let mut paths: Vec<String> = files.drain(..).map(|f| f.remote_path).collect();
    paths.sort();
    paths
}

#[tokio::test]
async fn upload_pdf_to_root_writes_three_files() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "report.pdf", b"%PDF-test-bytes-here");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uploaded.len(), 1);
    let u = &out.uploaded[0];
    assert_eq!(u.name, "report");
    assert_eq!(u.file_type, FileType::Pdf);
    assert_eq!(u.size_bytes, 20);
    assert_eq!(out.parent_path, "/");
    assert_eq!(out.parent_uuid, None);
    assert!(!out.dry_run);
    assert!(out.warnings.is_empty());

    let uuid = u.uuid;
    let raw: RawMetadata = serde_json::from_slice(
        &conn
            .read_file(&format!("{DATA_DIR}/{uuid}.metadata"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(raw.visible_name, "report");
    assert!(matches!(raw.item_type, ItemType::Document));
    assert_eq!(raw.parent, Parent::Root);
    assert!(!raw.deleted);
    assert_eq!(raw.version, 1);

    let content: RawContent = serde_json::from_slice(
        &conn
            .read_file(&format!("{DATA_DIR}/{uuid}.content"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(content.file_type, FileType::Pdf);

    let pdf_bytes = conn
        .read_file(&format!("{DATA_DIR}/{uuid}.pdf"))
        .await
        .unwrap();
    assert_eq!(pdf_bytes, b"%PDF-test-bytes-here");
}

#[tokio::test]
async fn upload_epub_to_subfolder_by_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let epub = make_local(dir.path(), "book.epub", b"PK-epub-stub");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&epub], Some("/Research"), None, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uploaded.len(), 1);
    assert_eq!(out.uploaded[0].file_type, FileType::Epub);
    assert_eq!(out.parent_path, "/Research");
    let parent_uuid = out.parent_uuid.unwrap();

    let uuid = out.uploaded[0].uuid;
    let raw: RawMetadata = serde_json::from_slice(
        &conn
            .read_file(&format!("{DATA_DIR}/{uuid}.metadata"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(raw.parent, Parent::Folder(parent_uuid));
}

#[tokio::test]
async fn upload_to_uuid_parent() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "x.pdf", b"%PDF-1.4");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], Some(FOLDER_RESEARCH_UUID), None, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.parent_path, "/Research");
    assert_eq!(out.parent_uuid.unwrap().to_string(), FOLDER_RESEARCH_UUID);
}

#[tokio::test]
async fn upload_with_custom_name_overrides_basename() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "raw.pdf", b"%PDF-test");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, Some("Final Draft"), false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uploaded[0].name, "Final Draft");
    let uuid = out.uploaded[0].uuid;
    let raw: RawMetadata = serde_json::from_slice(
        &conn
            .read_file(&format!("{DATA_DIR}/{uuid}.metadata"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(raw.visible_name, "Final Draft");
}

#[tokio::test]
async fn upload_multiple_files_writes_each() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let a = make_local(dir.path(), "a.pdf", b"AAAA");
    let b = make_local(dir.path(), "b.epub", b"BBBBBB");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&a, &b], None, None, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uploaded.len(), 2);
    assert_eq!(out.total_bytes, 10);
    let names: Vec<&str> = out.uploaded.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"b"));

    for u in &out.uploaded {
        let ext = match u.file_type {
            FileType::Pdf => "pdf",
            FileType::Epub => "epub",
            FileType::Notebook => unreachable!(),
        };
        assert!(
            conn.file_exists(&format!("{DATA_DIR}/{}.{ext}", u.uuid))
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn upload_dry_run_writes_nothing_and_skips_xochitl() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "skip.pdf", b"%PDF-1.4-skip");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, true),
        false,
    )
    .await
    .unwrap();

    assert!(out.dry_run);
    assert_eq!(out.uploaded.len(), 1);
    let uuid = out.uploaded[0].uuid;

    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{uuid}.metadata"))
            .await
            .unwrap()
    );
    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{uuid}.content"))
            .await
            .unwrap()
    );
    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{uuid}.pdf"))
            .await
            .unwrap()
    );
    assert!(
        !conn
            .executed_commands()
            .iter()
            .any(|c| c.contains("xochitl"))
    );
}

#[tokio::test]
async fn upload_invalid_extension_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let txt = make_local(dir.path(), "note.txt", b"hello");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&txt], None, None, false),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::FormatError(_)));
}

#[tokio::test]
async fn upload_missing_local_file_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let missing = PathBuf::from("/this/does/not/exist.pdf");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&missing], None, None, false),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::NotFound(_)));
}

#[tokio::test]
async fn upload_to_document_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "child.pdf", b"x");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], Some("/Paper Draft"), None, false),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn upload_to_trashed_folder_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "x.pdf", b"x");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], Some(FOLDER_TRASHED_UUID), None, false),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn upload_warns_on_duplicate_name() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "duplicate.pdf", b"%PDF-dup");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uploaded.len(), 1);
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("duplicate"));
}

#[tokio::test]
async fn upload_with_no_restart_skips_start_command() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "x.pdf", b"%PDF-noresume");

    upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        true,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl stop xochitl"));
    assert!(!cmds.iter().any(|c| c == "systemctl start xochitl"));
}

#[tokio::test]
async fn upload_normal_calls_stop_then_start_in_order() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "x.pdf", b"%PDF");

    upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        false,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    let stop_idx = cmds
        .iter()
        .position(|c| c == "systemctl stop xochitl")
        .expect("stop should be called");
    let start_idx = cmds
        .iter()
        .position(|c| c == "systemctl start xochitl")
        .expect("start should be called");
    assert!(stop_idx < start_idx, "stop must precede start");
}

#[tokio::test]
async fn upload_name_with_multiple_files_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let a = make_local(dir.path(), "a.pdf", b"a");
    let b = make_local(dir.path(), "b.pdf", b"b");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&a, &b], None, Some("OneName"), false),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn upload_writes_round_trippable_metadata() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "fresh.pdf", b"%PDF-fresh");

    let out = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], Some("/Research"), None, false),
        false,
    )
    .await
    .unwrap();

    let new_uuid = out.uploaded[0].uuid;
    let reloaded = build_tree(&conn).await;
    let entry = reloaded.get(&new_uuid).expect("new doc should be in tree");
    assert_eq!(entry.visible_name, "fresh");
    assert!(entry.is_document());
    assert_eq!(entry.parent, Parent::Folder(out.parent_uuid.unwrap()));
}

#[tokio::test]
async fn upload_source_write_failure_rolls_back_partial_files_and_restarts_xochitl() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let before = remote_files(&conn).await;
    conn.set_write_error_suffix(".pdf", "disk full");
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "fresh.pdf", b"%PDF-fresh");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains(".pdf"));
    assert_eq!(remote_files(&conn).await, before);
    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl stop xochitl"));
    assert!(cmds.iter().any(|c| c == "systemctl start xochitl"));
}

#[tokio::test]
async fn upload_content_write_failure_rolls_back_partial_files() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let before = remote_files(&conn).await;
    conn.set_write_error_suffix(".content", "permission denied");
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "fresh.pdf", b"%PDF-fresh");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("permission denied"));
    assert_eq!(remote_files(&conn).await, before);
}

#[tokio::test]
async fn upload_metadata_write_failure_rolls_back_partial_files() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let before = remote_files(&conn).await;
    conn.set_write_error_suffix(".metadata", "no space left");
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "fresh.pdf", b"%PDF-fresh");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf], None, None, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("no space left"));
    assert_eq!(remote_files(&conn).await, before);
}

#[tokio::test]
async fn upload_later_failure_keeps_earlier_successful_documents() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;
    let before = remote_files(&conn).await;
    conn.set_write_error_suffix(".epub", "remote full");
    let dir = tempfile::tempdir().unwrap();
    let pdf = make_local(dir.path(), "first.pdf", b"%PDF-first");
    let epub = make_local(dir.path(), "second.epub", b"PK-second");

    let err = upload::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(&[&pdf, &epub], None, None, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains(".epub"));
    let after = remote_files(&conn).await;
    assert_eq!(after.len(), before.len() + 3);

    let reloaded = build_tree(&conn).await;
    let names: Vec<&str> = reloaded
        .all_entries()
        .map(|entry| entry.visible_name.as_str())
        .collect();
    assert!(names.contains(&"first"));
    assert!(!names.contains(&"second"));
}
