//! Integration tests for `remarkable-cli read` — the one-step content command
//! that renders notebooks to PNG and downloads PDFs/ePubs, picking the action
//! by file type. Exercises the CLI plumbing over a `FakeConnection`; the
//! underlying render/download behaviour is covered by `tests/render.rs` and
//! `tests/download.rs`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use remarkable_cli::cli::ReadArgs;
use remarkable_cli::commands::read;
use remarkable_cli::error::CliError;
use remarkable_metadata::metadata::FileType;
use remarkable_metadata::page_range::PageSelection;
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::FakeConnection;
use remarkable_tablet::tablet::load_all_metadata;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const NOTEBOOK_UUID: &str = "cccccccc-1111-1111-1111-111111111111";
const PDF_UUID: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const FOLDER_UUID: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const PAGE_UUID_1: &str = "11111111-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE_UUID_2: &str = "22222222-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

const SMOKE_RM: &[u8] = include_bytes!("../../remarkable-rm/tests/fixtures/smoke.rm");
const PENS_SMALL_RM: &[u8] = include_bytes!("../../remarkable-rm/tests/fixtures/pens-small.rm");
const PDF_BYTES: &[u8] = b"%PDF-1.4 stub body";

fn populate(conn: &FakeConnection) {
    conn.mkdir(DATA_DIR);

    // Notebook "Sketches" with two pages.
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.metadata"),
        br#"{"visibleName":"Sketches","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.content"),
        format!(
            r#"{{"fileType":"notebook","pages":[{{"id":"{PAGE_UUID_1}"}},{{"id":"{PAGE_UUID_2}"}}]}}"#
        )
        .as_bytes(),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_1}.rm"),
        SMOKE_RM,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_2}.rm"),
        PENS_SMALL_RM,
    );

    // PDF "Paper Draft" with a source file.
    conn.set_file(
        &format!("{DATA_DIR}/{PDF_UUID}.metadata"),
        br#"{"visibleName":"Paper Draft","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{PDF_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    conn.set_file(&format!("{DATA_DIR}/{PDF_UUID}.pdf"), PDF_BYTES);

    // A folder, for the rejection test.
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_UUID}.metadata"),
        br#"{"visibleName":"Folder","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
}

async fn build_tree(conn: &FakeConnection) -> DocumentTree {
    let entries = load_all_metadata(conn, DATA_DIR).await.unwrap();
    DocumentTree::build(entries)
}

fn args(path_or_uuid: &str, output: Option<PathBuf>, pages: Option<&str>) -> ReadArgs {
    ReadArgs {
        path_or_uuid: path_or_uuid.to_string(),
        output,
        pages: pages.map(|s| PageSelection::from_str(s).expect("valid page spec")),
    }
}

fn downcast_cli(err: anyhow::Error) -> CliError {
    err.downcast::<CliError>().expect("expected CliError")
}

fn assert_png(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "{} is not a PNG",
        path.display()
    );
}

#[tokio::test]
async fn read_notebook_renders_pngs() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("sketches");

    let out = read::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), None),
    )
    .await
    .unwrap();

    assert_eq!(out.action, "rendered");
    assert_eq!(out.file_type, FileType::Notebook);
    assert_eq!(out.name, "Sketches");
    assert_eq!(out.output_path, out_dir);
    assert!(out.size_bytes.is_none());
    assert_eq!(out.artifacts.len(), 2);
    for (idx, artifact) in out.artifacts.iter().enumerate() {
        assert_eq!(artifact.page, Some(u32::try_from(idx + 1).unwrap()));
        assert_png(&artifact.path);
    }
}

#[tokio::test]
async fn read_notebook_honours_pages_filter() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("subset");

    let out = read::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), Some("2")),
    )
    .await
    .unwrap();

    assert_eq!(out.artifacts.len(), 1);
    assert_eq!(out.artifacts[0].page, Some(2));
    assert_png(&out.artifacts[0].path);
}

#[tokio::test]
async fn read_pdf_downloads_source() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_file = dest.path().join("paper.pdf");

    let out = read::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(PDF_UUID, Some(out_file.clone()), None),
    )
    .await
    .unwrap();

    assert_eq!(out.action, "downloaded");
    assert_eq!(out.file_type, FileType::Pdf);
    assert_eq!(out.name, "Paper Draft");
    assert_eq!(out.output_path, out_file);
    assert_eq!(out.size_bytes, Some(PDF_BYTES.len() as u64));
    assert_eq!(out.artifacts.len(), 1);
    assert_eq!(out.artifacts[0].page, None);
    assert_eq!(std::fs::read(&out_file).unwrap(), PDF_BYTES);
}

#[tokio::test]
async fn read_pdf_rejects_pages_flag() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();

    let err = read::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(PDF_UUID, Some(dest.path().join("nope.pdf")), Some("1")),
    )
    .await
    .unwrap_err();
    match downcast_cli(err) {
        CliError::InvalidPath(msg) => assert!(msg.contains("pages"), "msg = {msg}"),
        other => panic!("expected InvalidPath, got {other:?}"),
    }
}

#[tokio::test]
async fn read_folder_is_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();

    let err = read::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args("/Folder", Some(dest.path().join("nope")), None),
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn read_root_is_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();

    let err = read::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args("/", Some(dest.path().join("nope")), None),
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}
