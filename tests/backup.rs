use std::time::{Duration, SystemTime};

use remarkable_tablet_cli_rs::cli::BackupArgs;
use remarkable_tablet_cli_rs::commands::backup::{self, BackupManifest};
use remarkable_tablet_cli_rs::connection::FakeConnection;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const FOLDER_UUID: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const DOC_PDF_UUID: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const NOTEBOOK_UUID: &str = "cccccccc-1111-1111-1111-111111111111";
const PAGE_UUID_1: &str = "11111111-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE_UUID_2: &str = "22222222-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

fn populate_xochitl(conn: &FakeConnection) {
    conn.mkdir(DATA_DIR);

    // Folder
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_UUID}.metadata"),
        br#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );

    // PDF doc
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PDF_UUID}.metadata"),
        br#"{"visibleName":"Paper","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PDF_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_PDF_UUID}.pdf"),
        b"%PDF-1.4 stub bytes",
    );

    // Notebook with 2 pages
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}.metadata"),
        br#"{"visibleName":"Notes","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710700000000,"metadatamodified":1710700000000,"version":1}"#,
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
        b"page1bytes",
    );
    conn.set_file(
        &format!("{DATA_DIR}/{NOTEBOOK_UUID}/{PAGE_UUID_2}.rm"),
        b"page2bytes!",
    );

    // /etc/version for firmware detection
    conn.set_file("/etc/version", b"20240101.123456\n");
}

fn args(local_dir: &std::path::Path, incremental: bool, dry_run: bool) -> BackupArgs {
    BackupArgs {
        local_dir: local_dir.to_path_buf(),
        incremental,
        dry_run,
    }
}

#[tokio::test]
async fn full_backup_copies_every_file_and_writes_manifest() {
    let conn = FakeConnection::new();
    populate_xochitl(&conn);
    let dest = tempfile::tempdir().unwrap();

    let out = backup::run_with_conn(
        &conn,
        DATA_DIR,
        dest.path(),
        &args(dest.path(), false, false),
    )
    .await
    .unwrap();

    assert!(!out.dry_run);
    assert!(!out.incremental);
    // 1 folder.metadata + (doc.metadata+content+pdf) + (nb.metadata+content + 2 page.rm) = 8
    assert_eq!(out.file_count, 8);
    assert_eq!(out.copied, 8);
    assert_eq!(out.skipped, 0);
    assert_eq!(out.firmware_version.as_deref(), Some("20240101.123456"));

    // Verify on-disk layout
    let xochitl = dest.path().join("xochitl");
    assert!(
        xochitl.join(format!("{DOC_PDF_UUID}.pdf")).exists(),
        "PDF should be copied"
    );
    assert!(
        xochitl
            .join(format!("{NOTEBOOK_UUID}/{PAGE_UUID_1}.rm"))
            .exists(),
        "page 1 should be copied under nested dir"
    );
    assert_eq!(
        std::fs::read(xochitl.join(format!("{NOTEBOOK_UUID}/{PAGE_UUID_2}.rm"))).unwrap(),
        b"page2bytes!"
    );

    // Firmware version saved
    assert_eq!(
        std::fs::read(dest.path().join("version")).unwrap(),
        b"20240101.123456\n"
    );

    // Manifest JSON
    let manifest_path = dest.path().join("backup_manifest.json");
    assert!(manifest_path.exists());
    let manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(
        manifest.firmware_version.as_deref(),
        Some("20240101.123456")
    );
    assert_eq!(manifest.file_count, 8);
    assert!(
        manifest
            .files
            .iter()
            .any(|f| f.path == format!("{DOC_PDF_UUID}.pdf"))
    );
    assert!(
        manifest
            .files
            .iter()
            .any(|f| f.path == format!("{NOTEBOOK_UUID}/{PAGE_UUID_1}.rm"))
    );
}

#[tokio::test]
async fn dry_run_does_not_write_anything() {
    let conn = FakeConnection::new();
    populate_xochitl(&conn);
    let dest = tempfile::tempdir().unwrap();

    let out = backup::run_with_conn(
        &conn,
        DATA_DIR,
        dest.path(),
        &args(dest.path(), false, true),
    )
    .await
    .unwrap();

    assert!(out.dry_run);
    assert_eq!(out.copied, 8);
    assert_eq!(out.skipped, 0);
    assert!(out.manifest_path.is_none());
    // Nothing written to disk
    assert!(!dest.path().join("xochitl").exists());
    assert!(!dest.path().join("version").exists());
    assert!(!dest.path().join("backup_manifest.json").exists());
}

#[tokio::test]
async fn empty_xochitl_succeeds() {
    let conn = FakeConnection::new();
    conn.mkdir(DATA_DIR);
    conn.set_file("/etc/version", b"20240101");
    let dest = tempfile::tempdir().unwrap();

    let out = backup::run_with_conn(
        &conn,
        DATA_DIR,
        dest.path(),
        &args(dest.path(), false, false),
    )
    .await
    .unwrap();

    assert_eq!(out.file_count, 0);
    assert_eq!(out.copied, 0);
    let manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(dest.path().join("backup_manifest.json")).unwrap())
            .unwrap();
    assert!(manifest.files.is_empty());
    assert_eq!(manifest.total_bytes, 0);
}

#[tokio::test]
async fn incremental_skips_local_files_with_newer_or_equal_mtime() {
    let conn = FakeConnection::new();
    populate_xochitl(&conn);

    // Force the remote PDF to a fixed older mtime so we can compare.
    let remote_pdf_path = format!("{DATA_DIR}/{DOC_PDF_UUID}.pdf");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    conn.set_file_with_mtime(&remote_pdf_path, b"%PDF-1.4 stub bytes", old);

    let dest = tempfile::tempdir().unwrap();
    let xochitl_local = dest.path().join("xochitl");
    std::fs::create_dir_all(&xochitl_local).unwrap();
    // Create a local copy of the PDF with a newer mtime.
    let local_pdf = xochitl_local.join(format!("{DOC_PDF_UUID}.pdf"));
    std::fs::write(&local_pdf, b"local stale bytes").unwrap();
    let newer = SystemTime::UNIX_EPOCH + Duration::from_secs(1_710_000_000);
    filetime::set_file_mtime(&local_pdf, filetime::FileTime::from_system_time(newer)).unwrap();

    let out = backup::run_with_conn(
        &conn,
        DATA_DIR,
        dest.path(),
        &args(dest.path(), true, false),
    )
    .await
    .unwrap();

    assert!(out.incremental);
    // file_count is the total walked, copied is what survived the filter.
    assert_eq!(out.file_count, 8);
    assert_eq!(out.copied, 7); // PDF was skipped because local is newer
    assert_eq!(out.skipped, 1);
    // The local file should NOT have been overwritten.
    assert_eq!(std::fs::read(&local_pdf).unwrap(), b"local stale bytes");
}

#[tokio::test]
async fn incremental_re_fetches_local_files_with_older_mtime() {
    let conn = FakeConnection::new();
    populate_xochitl(&conn);

    // Make the remote PDF newer than the local copy.
    let remote_pdf_path = format!("{DATA_DIR}/{DOC_PDF_UUID}.pdf");
    let new_remote = SystemTime::UNIX_EPOCH + Duration::from_secs(1_720_000_000);
    conn.set_file_with_mtime(&remote_pdf_path, b"new remote bytes", new_remote);

    let dest = tempfile::tempdir().unwrap();
    let xochitl_local = dest.path().join("xochitl");
    std::fs::create_dir_all(&xochitl_local).unwrap();
    let local_pdf = xochitl_local.join(format!("{DOC_PDF_UUID}.pdf"));
    std::fs::write(&local_pdf, b"local old bytes").unwrap();
    let older = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    filetime::set_file_mtime(&local_pdf, filetime::FileTime::from_system_time(older)).unwrap();

    let out = backup::run_with_conn(
        &conn,
        DATA_DIR,
        dest.path(),
        &args(dest.path(), true, false),
    )
    .await
    .unwrap();

    assert!(out.incremental);
    assert_eq!(std::fs::read(&local_pdf).unwrap(), b"new remote bytes");
    assert!(out.copied >= 1);
}

#[tokio::test]
async fn incremental_first_run_acts_like_full_backup() {
    let conn = FakeConnection::new();
    populate_xochitl(&conn);
    let dest = tempfile::tempdir().unwrap();

    let out = backup::run_with_conn(
        &conn,
        DATA_DIR,
        dest.path(),
        &args(dest.path(), true, false),
    )
    .await
    .unwrap();

    assert_eq!(out.copied, out.file_count);
    assert_eq!(out.skipped, 0);
}
