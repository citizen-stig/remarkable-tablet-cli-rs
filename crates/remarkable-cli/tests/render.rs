//! Integration tests for `remarkable-cli render`.
//!
//! Exercises both source modes:
//! - `run_with_conn` over a `FakeConnection` populated with v6 fixtures
//!   (mirrors `tests/download.rs`).
//! - `--from-backup` over a real `tempfile::TempDir` laid out the same way
//!   the `backup` command would write it.
//!
//! Visual fidelity is asserted by `crates/remarkable-rm/tests/render.rs`;
//! these tests just verify the CLI plumbing — the right files end up on
//! disk with sensible names and dimensions.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use remarkable_cli::cli::RenderArgs;
use remarkable_cli::commands::render;
use remarkable_cli::error::CliError;
use remarkable_metadata::page_range::PageSelection;
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::FakeConnection;
use remarkable_tablet::tablet::load_all_metadata;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const NOTEBOOK_UUID: &str = "cccccccc-1111-1111-1111-111111111111";
const NOTEBOOK_GAPPED_UUID: &str = "cccccccc-2222-2222-2222-222222222222";
const PDF_UUID: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const PAGE_UUID_1: &str = "11111111-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE_UUID_2: &str = "22222222-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE_UUID_3: &str = "33333333-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const GAPPED_PAGE_UUID_1: &str = "44444444-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const GAPPED_PAGE_UUID_2: &str = "55555555-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const GAPPED_PAGE_UUID_3: &str = "66666666-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

const SMOKE_RM: &[u8] = include_bytes!("../../remarkable-rm/tests/fixtures/smoke.rm");
const PENS_SMALL_RM: &[u8] = include_bytes!("../../remarkable-rm/tests/fixtures/pens-small.rm");
const LAYERS_RM: &[u8] = include_bytes!("../../remarkable-rm/tests/fixtures/layers.rm");

fn populate(conn: &FakeConnection) {
    conn.mkdir(DATA_DIR);

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
        SMOKE_RM,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_2}.rm"),
        PENS_SMALL_RM,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_3}.rm"),
        LAYERS_RM,
    );

    // Notebook whose middle page is listed in `.content` but missing on disk.
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_GAPPED_UUID}.metadata"),
        br#"{"visibleName":"Gapped Sketches","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_GAPPED_UUID}.content"),
        format!(
            r#"{{"fileType":"notebook","pages":[{{"id":"{GAPPED_PAGE_UUID_1}"}},{{"id":"{GAPPED_PAGE_UUID_2}"}},{{"id":"{GAPPED_PAGE_UUID_3}"}}]}}"#
        )
        .as_bytes(),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_GAPPED_UUID}/{GAPPED_PAGE_UUID_1}.rm"),
        SMOKE_RM,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_GAPPED_UUID}/{GAPPED_PAGE_UUID_3}.rm"),
        LAYERS_RM,
    );

    // A non-notebook for the rejection test.
    conn.set_file(
        &format!("{DATA_DIR}/{PDF_UUID}.metadata"),
        br#"{"visibleName":"Paper Draft","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{PDF_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    conn.set_file(&format!("{DATA_DIR}/{PDF_UUID}.pdf"), b"%PDF-stub");
}

async fn build_tree(conn: &FakeConnection) -> DocumentTree {
    let entries = load_all_metadata(conn, DATA_DIR).await.unwrap();
    DocumentTree::build(entries)
}

fn args(
    path_or_uuid: &str,
    output: Option<PathBuf>,
    pages: Option<&str>,
    from_backup: Option<PathBuf>,
) -> RenderArgs {
    RenderArgs {
        path_or_uuid: path_or_uuid.to_string(),
        output,
        pages: pages.map(|s| PageSelection::from_str(s).expect("valid page spec")),
        width: remarkable_rm::DEFAULT_WIDTH,
        dpi: 226,
        from_backup,
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
async fn render_notebook_writes_one_png_per_page() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("sketches");

    let out = render::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), None, None),
    )
    .await
    .unwrap();

    assert_eq!(out.uuid.to_string(), NOTEBOOK_UUID);
    assert_eq!(out.name, "Sketches");
    assert_eq!(out.pages.len(), 3);
    for (idx, page) in out.pages.iter().enumerate() {
        let expected_idx = u32::try_from(idx + 1).unwrap();
        assert_eq!(page.page, expected_idx);
        let expected_path = out_dir.join(format!("{NOTEBOOK_UUID}_page_{expected_idx}.png"));
        assert_eq!(page.output_path, expected_path);
        assert_png(&expected_path);
    }
}

#[tokio::test]
async fn render_with_pages_filter_picks_only_selected() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("subset");

    let out = render::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(out_dir.clone()), Some("1,3"), None),
    )
    .await
    .unwrap();

    assert_eq!(out.pages.len(), 2);
    let page_indices: Vec<u32> = out.pages.iter().map(|p| p.page).collect();
    assert_eq!(page_indices, vec![1, 3]);
    assert_png(&out_dir.join(format!("{NOTEBOOK_UUID}_page_1.png")));
    assert!(!out_dir.join(format!("{NOTEBOOK_UUID}_page_2.png")).exists());
    assert_png(&out_dir.join(format!("{NOTEBOOK_UUID}_page_3.png")));
}

#[tokio::test]
async fn render_pdf_is_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let err = render::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(PDF_UUID, Some(dest.path().join("nope")), None, None),
    )
    .await
    .unwrap_err();
    match downcast_cli(err) {
        CliError::InvalidPath(msg) => assert!(msg.contains("notebook"), "msg = {msg}"),
        other => panic!("expected InvalidPath, got {other:?}"),
    }
}

#[tokio::test]
async fn render_root_is_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let err = render::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args("/", Some(dest.path().join("nope")), None, None),
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn render_existing_output_is_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let collide = dest.path().join("already-here");
    std::fs::create_dir(&collide).unwrap();
    let err = render::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_UUID, Some(collide), None, None),
    )
    .await
    .unwrap_err();
    assert!(matches!(downcast_cli(err), CliError::AlreadyExists(_)));
}

#[tokio::test]
async fn render_custom_width_is_honoured() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("narrow");

    let mut a = args(NOTEBOOK_UUID, Some(out_dir.clone()), Some("1"), None);
    a.width = 702;

    let out = render::run_with_conn(&conn, DATA_DIR, &tree, &a)
        .await
        .unwrap();
    assert_eq!(out.pages.len(), 1);
    assert_eq!(out.width, 702);
}

#[tokio::test]
async fn render_pages_filter_preserves_recorded_page_numbers_when_files_are_missing() {
    let conn = FakeConnection::new();
    populate(&conn);
    let tree = build_tree(&conn).await;
    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("gapped");

    let out = render::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &args(NOTEBOOK_GAPPED_UUID, Some(out_dir.clone()), Some("3"), None),
    )
    .await
    .unwrap();

    assert_eq!(out.pages.len(), 1);
    assert_eq!(out.pages[0].page, 3);
    let expected_path = out_dir.join(format!("{NOTEBOOK_GAPPED_UUID}_page_3.png"));
    assert_eq!(out.pages[0].output_path, expected_path);
    assert_png(&out.pages[0].output_path);
    assert!(
        !out_dir
            .join(format!("{NOTEBOOK_GAPPED_UUID}_page_2.png"))
            .exists()
    );
}

#[tokio::test]
async fn render_from_backup_reads_local_directory() {
    // Mirror the layout `backup` produces: <root>/xochitl/<files>.
    let backup = tempfile::tempdir().unwrap();
    let xochitl = backup.path().join("xochitl");
    std::fs::create_dir_all(&xochitl).unwrap();
    std::fs::write(
        xochitl.join(format!("{NOTEBOOK_UUID}.metadata")),
        r#"{"visibleName":"Sketches","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    )
    .unwrap();
    std::fs::write(
        xochitl.join(format!("{NOTEBOOK_UUID}.content")),
        format!(
            r#"{{"fileType":"notebook","pages":[{{"id":"{PAGE_UUID_1}"}},{{"id":"{PAGE_UUID_2}"}}]}}"#
        ),
    )
    .unwrap();
    let pages_dir = xochitl.join(NOTEBOOK_UUID);
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(pages_dir.join(format!("{PAGE_UUID_1}.rm")), SMOKE_RM).unwrap();
    std::fs::write(pages_dir.join(format!("{PAGE_UUID_2}.rm")), PENS_SMALL_RM).unwrap();

    let dest_dir = tempfile::tempdir().unwrap();
    let out_dir = dest_dir.path().join("sketches");
    let out = render::run_from_backup(
        backup.path(),
        &args(NOTEBOOK_UUID, Some(out_dir), None, None),
    )
    .await
    .unwrap();
    assert_eq!(out.pages.len(), 2);
    for page in &out.pages {
        assert_png(&page.output_path);
    }
}

#[tokio::test]
async fn render_from_backup_accepts_root_pointing_to_xochitl_directly() {
    // The user might pass the xochitl tree itself rather than its parent.
    let backup = tempfile::tempdir().unwrap();
    let xochitl = backup.path();
    std::fs::write(
        xochitl.join(format!("{NOTEBOOK_UUID}.metadata")),
        r#"{"visibleName":"Sketches","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710800000000,"metadatamodified":1710800000000,"version":1}"#,
    )
    .unwrap();
    std::fs::write(
        xochitl.join(format!("{NOTEBOOK_UUID}.content")),
        format!(r#"{{"fileType":"notebook","pages":[{{"id":"{PAGE_UUID_1}"}}]}}"#),
    )
    .unwrap();
    let pages_dir = xochitl.join(NOTEBOOK_UUID);
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(pages_dir.join(format!("{PAGE_UUID_1}.rm")), SMOKE_RM).unwrap();

    let dest = tempfile::tempdir().unwrap();
    let out_dir = dest.path().join("sketches");
    let out = render::run_from_backup(xochitl, &args(NOTEBOOK_UUID, Some(out_dir), None, None))
        .await
        .unwrap();
    assert_eq!(out.pages.len(), 1);
    assert_png(&out.pages[0].output_path);
}
