use std::path::PathBuf;
use std::str::FromStr;

use remarkable_tablet_cli_rs::cli::DownloadArgs;
use remarkable_tablet_cli_rs::commands::download;
use remarkable_tablet_cli_rs::connection::FakeConnection;
use remarkable_tablet_cli_rs::error::CliError;
use remarkable_tablet_cli_rs::metadata::FileType;
use remarkable_tablet_cli_rs::page_range::PageSelection;
use remarkable_tablet_cli_rs::tablet::load_all_metadata;
use remarkable_tablet_cli_rs::tree::DocumentTree;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const FOLDER_UUID: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const DOC_PDF_UUID: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const DOC_EPUB_UUID: &str = "bbbbbbbb-2222-2222-2222-222222222222";
const NOTEBOOK_UUID: &str = "cccccccc-1111-1111-1111-111111111111";
const PAGE_UUID_1: &str = "11111111-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE_UUID_2: &str = "22222222-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE_UUID_3: &str = "33333333-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const NOTEBOOK_NAMED_SLASH_UUID: &str = "dddddddd-1111-1111-1111-111111111111";
const EMPTY_NOTEBOOK_UUID: &str = "eeeeeeee-1111-1111-1111-111111111111";
const NOTEBOOK_MISSING_DIR_UUID: &str = "ffffffff-1111-1111-1111-111111111111";

fn populate(conn: &FakeConnection) {
    conn.mkdir(DATA_DIR);

    // Folder so download-of-folder test has something to target.
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_UUID}.metadata"),
        br#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );

    // PDF document at root.
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PDF_UUID}.metadata"),
        br#"{"visibleName":"Paper Draft","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PDF_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PDF_UUID}.pdf"),
        b"%PDF-stub-bytes-here",
    );

    // ePub document at root.
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_EPUB_UUID}.metadata"),
        br#"{"visibleName":"Quick Read","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_EPUB_UUID}.content"),
        br#"{"fileType":"epub"}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_EPUB_UUID}.epub"),
        b"PK-zip-stub-bytes",
    );

    // Notebook with 3 pages, ordered via .content.
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.metadata"),
        br#"{"visibleName":"Sketches","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.content"),
        format!(
            r#"{{"fileType":"notebook","pages":[{{"id":"{PAGE_UUID_1}"}},{{"id":"{PAGE_UUID_2}"}},{{"id":"{PAGE_UUID_3}"}}]}}"#
        )
        .as_bytes(),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_1}.rm"),
        b"page1",
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_2}.rm"),
        b"page2bytes",
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_3}.rm"),
        b"page3andmorebytes",
    );

    // Notebook whose visible name contains a `/` (sanitization edge case).
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_NAMED_SLASH_UUID}.metadata"),
        br#"{"visibleName":"a/b","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_NAMED_SLASH_UUID}.content"),
        format!(r#"{{"fileType":"notebook","pages":[{{"id":"{PAGE_UUID_1}"}}]}}"#).as_bytes(),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_NAMED_SLASH_UUID}/{PAGE_UUID_1}.rm"),
        b"slash",
    );

    // Empty notebook (zero recorded pages, no on-disk page directory).
    conn.set_file(
        &format!("{DATA_DIR}/{EMPTY_NOTEBOOK_UUID}.metadata"),
        br#"{"visibleName":"Empty","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{EMPTY_NOTEBOOK_UUID}.content"),
        br#"{"fileType":"notebook","pages":[]}"#,
    );

    // Notebook whose `.content` records pages but whose page directory is
    // missing, used to confirm we don't silently treat that as empty.
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_MISSING_DIR_UUID}.metadata"),
        br#"{"visibleName":"Missing Dir","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_MISSING_DIR_UUID}.content"),
        format!(r#"{{"fileType":"notebook","pages":[{{"id":"{PAGE_UUID_1}"}}]}}"#).as_bytes(),
    );
}

async fn build_tree(conn: &FakeConnection) -> DocumentTree {
    let entries = load_all_metadata(conn, DATA_DIR).await.unwrap();
    DocumentTree::build(entries)
}

fn args(path_or_uuid: &str, output: Option<PathBuf>, pages: Option<&str>) -> DownloadArgs {
    DownloadArgs {
        path_or_uuid: path_or_uuid.to_string(),
        output,
        pages: pages.map(|s| PageSelection::from_str(s).expect("valid page spec")),
    }
}

fn downcast_cli(err: anyhow::Error) -> CliError {
    err.downcast::<CliError>().expect("expected CliError")
}

#[tokio::test]
async fn download_pdf_by_path_writes_file() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();
    let out_path = dest_dir.path().join("paper.pdf");

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args("/Paper Draft", Some(out_path.clone()), None),
    )
    .await
    .unwrap();

    assert_eq!(out.file_type, FileType::Pdf);
    assert_eq!(out.size_bytes, 20);
    assert!(out.pages_written.is_none());
    assert_eq!(std::fs::read(&out_path).unwrap(), b"%PDF-stub-bytes-here");
}

#[tokio::test]
async fn download_epub_by_uuid_writes_file() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();
    let out_path = dest_dir.path().join("read.epub");

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(DOC_EPUB_UUID, Some(out_path.clone()), None),
    )
    .await
    .unwrap();

    assert_eq!(out.file_type, FileType::Epub);
    assert_eq!(std::fs::read(&out_path).unwrap(), b"PK-zip-stub-bytes");
}

#[tokio::test]
async fn download_notebook_writes_all_pages_in_order() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();
    let out_dir = dest_dir.path().join("sketches");

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), None),
    )
    .await
    .unwrap();

    assert_eq!(out.file_type, FileType::Notebook);
    assert_eq!(out.pages_written, Some(3));
    assert!(out_dir.join(format!("{PAGE_UUID_1}.rm")).exists());
    assert!(out_dir.join(format!("{PAGE_UUID_2}.rm")).exists());
    assert!(out_dir.join(format!("{PAGE_UUID_3}.rm")).exists());
    let total: u64 = ["page1", "page2bytes", "page3andmorebytes"]
        .iter()
        .map(|s| s.len() as u64)
        .sum();
    assert_eq!(out.size_bytes, total);
}

#[tokio::test]
async fn download_notebook_with_pages_filter() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();
    let out_dir = dest_dir.path().join("subset");

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), Some("1,3")),
    )
    .await
    .unwrap();

    assert_eq!(out.pages_written, Some(2));
    assert!(out_dir.join(format!("{PAGE_UUID_1}.rm")).exists());
    assert!(!out_dir.join(format!("{PAGE_UUID_2}.rm")).exists());
    assert!(out_dir.join(format!("{PAGE_UUID_3}.rm")).exists());
}

#[tokio::test]
async fn download_pages_on_pdf_is_invalid_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();

    let err = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(
            "/Paper Draft",
            Some(dest_dir.path().join("x.pdf")),
            Some("1-2"),
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn download_folder_is_invalid_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;

    let err = download::run_with_conn(&conn, DATA_DIR, &tree, &args("/Work", None, None))
        .await
        .unwrap_err();
    let cli = downcast_cli(err);
    assert!(matches!(cli, CliError::InvalidPath(_)));
    assert!(cli.to_string().contains("folder"));
}

#[tokio::test]
async fn download_root_is_invalid_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;

    let err = download::run_with_conn(&conn, DATA_DIR, &tree, &args("/", None, None))
        .await
        .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn download_existing_output_path_errors() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();
    let out_path = dest_dir.path().join("collision.pdf");
    std::fs::write(&out_path, b"already there").unwrap();

    let err = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args("/Paper Draft", Some(out_path), None),
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::AlreadyExists(_)));
}

#[tokio::test]
async fn download_notebook_with_slash_in_name_sanitizes_default_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;

    // Run from a tempdir so the implicit `./a_b` directory doesn't pollute repo root.
    let cwd = tempfile::tempdir().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(cwd.path()).unwrap();

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_NAMED_SLASH_UUID, None, None),
    )
    .await;

    let restore = std::env::set_current_dir(&prev);

    let out = out.expect("download should succeed");
    assert_eq!(out.output_path, PathBuf::from("a_b"));
    assert!(
        cwd.path()
            .join("a_b")
            .join(format!("{PAGE_UUID_1}.rm"))
            .exists()
    );
    restore.unwrap();
}

#[tokio::test]
async fn download_empty_notebook_creates_empty_dir() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();
    let out_dir = dest_dir.path().join("empty");

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(EMPTY_NOTEBOOK_UUID, Some(out_dir.clone()), None),
    )
    .await
    .unwrap();
    assert_eq!(out.pages_written, Some(0));
    assert_eq!(out.size_bytes, 0);
    assert!(out_dir.exists() && out_dir.is_dir());
    assert!(std::fs::read_dir(&out_dir).unwrap().next().is_none());
}

#[tokio::test]
async fn download_nonempty_notebook_with_missing_page_dir_fails() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest_dir = tempfile::tempdir().unwrap();

    let err = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(
            NOTEBOOK_MISSING_DIR_UUID,
            Some(dest_dir.path().join("missing-dir")),
            None,
        ),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains(NOTEBOOK_MISSING_DIR_UUID));
}

#[tokio::test]
async fn download_notebook_page_dir_read_error_fails() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    conn.set_read_dir_error(&format!("{DATA_DIR}/{NOTEBOOK_UUID}"), "permission denied");
    let dest_dir = tempfile::tempdir().unwrap();
    let out_dir = dest_dir.path().join("sketches");

    let err = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), None),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains(&format!("{DATA_DIR}/{NOTEBOOK_UUID}"))
    );
    assert!(!out_dir.exists());
}

#[tokio::test]
async fn download_notebook_pages_requires_readable_content() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    conn.set_read_error(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.content"),
        "permission denied",
    );
    let dest_dir = tempfile::tempdir().unwrap();

    let err = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(
            NOTEBOOK_UUID,
            Some(dest_dir.path().join("subset")),
            Some("1-2"),
        ),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("readable notebook page order"));
}

#[tokio::test]
async fn download_notebook_pages_requires_pages_array() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.content"),
        br#"{"fileType":"notebook","pageCount":3}"#,
    );
    let dest_dir = tempfile::tempdir().unwrap();

    let err = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(
            NOTEBOOK_UUID,
            Some(dest_dir.path().join("subset")),
            Some("1-2"),
        ),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("pages array"));
}

#[tokio::test]
async fn download_notebook_without_pages_filter_tolerates_unreadable_content() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    conn.set_read_error(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.content"),
        "permission denied",
    );
    let dest_dir = tempfile::tempdir().unwrap();
    let out_dir = dest_dir.path().join("all-pages");

    let out = download::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), None),
    )
    .await
    .unwrap();

    assert_eq!(out.pages_written, Some(3));
    assert!(out_dir.join(format!("{PAGE_UUID_1}.rm")).exists());
    assert!(out_dir.join(format!("{PAGE_UUID_2}.rm")).exists());
    assert!(out_dir.join(format!("{PAGE_UUID_3}.rm")).exists());
}
