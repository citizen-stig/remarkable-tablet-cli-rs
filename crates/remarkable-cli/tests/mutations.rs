//! Integration tests for the file-organization commands: `mkdir`, `mv`,
//! `rename`, and `rm`.
//!
//! Mirrors `tests/upload.rs` in shape: a shared `populate` lays down a small
//! fixture tree, `register_xochitl` wires up the systemctl no-op outputs, and
//! each test runs one command against `FakeConnection` and asserts the visible
//! metadata + executed command sequence.

use remarkable_cli::cli::{MkdirArgs, MvArgs, RenameArgs, RmArgs};
use remarkable_cli::commands::{mkdir, mv, rename, rm};
use remarkable_cli::error::CliError;
use remarkable_metadata::metadata::{ItemType, Parent, RawMetadata};
use remarkable_metadata::tree::DocumentTree;
use remarkable_tablet::connection::{FakeConnection, TabletConnection};
use remarkable_tablet::tablet::load_all_metadata;

const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

const FOLDER_WORK_UUID: &str = "aaaaaaaa-1111-1111-1111-111111111111";
const FOLDER_SUB_UUID: &str = "aaaaaaaa-2222-2222-2222-222222222222";
const FOLDER_TRASHED_UUID: &str = "aaaaaaaa-3333-3333-3333-333333333333";
const DOC_NOTES_UUID: &str = "bbbbbbbb-1111-1111-1111-111111111111";
const DOC_QUICK_UUID: &str = "bbbbbbbb-2222-2222-2222-222222222222";
const DOC_DUP_UUID: &str = "bbbbbbbb-3333-3333-3333-333333333333";

fn populate(conn: &FakeConnection) {
    conn.mkdir(DATA_DIR);

    // /Work
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_WORK_UUID}.metadata"),
        br#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );
    // /Work/Sub
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_SUB_UUID}.metadata"),
        format!(
            r#"{{"visibleName":"Sub","type":"CollectionType","parent":"{FOLDER_WORK_UUID}","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}}"#
        )
        .as_bytes(),
    );
    // Trashed folder.
    conn.set_file(
        &format!("{DATA_DIR}/{FOLDER_TRASHED_UUID}.metadata"),
        br#"{"visibleName":"Old Folder","type":"CollectionType","parent":"trash","deleted":true,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
    );
    // /Work/Notes (PDF)
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES_UUID}.metadata"),
        format!(
            r#"{{"visibleName":"Notes","type":"DocumentType","parent":"{FOLDER_WORK_UUID}","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1,"futureField":{{"k":"v"}}}}"#
        )
        .as_bytes(),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES_UUID}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    // /Quick Note (notebook)
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_QUICK_UUID}.metadata"),
        br#"{"visibleName":"Quick Note","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_QUICK_UUID}.content"),
        br#"{"fileType":"notebook"}"#,
    );
    // /duplicate (PDF) — used by duplicate-name warning tests.
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

fn downcast_cli(err: anyhow::Error) -> CliError {
    err.downcast::<CliError>().expect("expected CliError")
}

async fn read_metadata(conn: &FakeConnection, uuid: &str) -> RawMetadata {
    let raw = conn
        .read_file(&format!("{DATA_DIR}/{uuid}.metadata"))
        .await
        .unwrap();
    serde_json::from_slice(&raw).unwrap()
}

async fn read_metadata_value(conn: &FakeConnection, uuid: &str) -> serde_json::Value {
    let raw = conn
        .read_file(&format!("{DATA_DIR}/{uuid}.metadata"))
        .await
        .unwrap();
    serde_json::from_slice(&raw).unwrap()
}

// ---------------------------------------------------------------------------
// mkdir
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mkdir_creates_root_level_folder() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/Reports".into(),
            parents: false,
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.created.len(), 1);
    let c = &out.created[0];
    assert_eq!(c.name, "Reports");
    assert_eq!(c.path, "/Reports");
    assert!(c.parent_uuid.is_none());

    let raw = read_metadata(&conn, &c.uuid.to_string()).await;
    assert_eq!(raw.visible_name, "Reports");
    assert!(matches!(raw.item_type, ItemType::Collection));
    assert_eq!(raw.parent, Parent::Root);
    assert!(!raw.deleted);
    assert_eq!(raw.version, 1);

    // No .content file for folders.
    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{}.content", c.uuid))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn mkdir_parents_creates_full_chain() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/A/B/C".into(),
            parents: true,
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.created.len(), 3);
    let by_path: std::collections::HashMap<_, _> =
        out.created.iter().map(|c| (c.path.as_str(), c)).collect();

    let a = by_path["/A"];
    let b = by_path["/A/B"];
    let c = by_path["/A/B/C"];

    assert!(a.parent_uuid.is_none());
    assert_eq!(b.parent_uuid, Some(a.uuid));
    assert_eq!(c.parent_uuid, Some(b.uuid));

    // Reload the tree and verify they connect.
    let reloaded = build_tree(&conn).await;
    assert_eq!(
        reloaded.get(&c.uuid).unwrap().parent,
        Parent::Folder(b.uuid)
    );
    assert_eq!(reloaded.display_path(&c.uuid), Some("/A/B/C"));
}

#[tokio::test]
async fn mkdir_parents_with_existing_full_path_is_no_op() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/Work/Sub".into(),
            parents: true,
        },
        false,
    )
    .await
    .unwrap();

    assert!(out.created.is_empty());
    // No xochitl bracket on a no-op.
    let cmds = conn.executed_commands();
    assert!(!cmds.iter().any(|c| c.contains("xochitl")));
}

#[tokio::test]
async fn mkdir_existing_path_without_parents_errors_already_exists() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/Work".into(),
            parents: false,
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::AlreadyExists(_)));
}

#[tokio::test]
async fn mkdir_missing_intermediate_without_parents_errors_not_found() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/Work/NewSub/Inner".into(),
            parents: false,
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::NotFound(_)));
}

#[tokio::test]
async fn mkdir_through_document_errors_invalid_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/Quick Note/Child".into(),
            parents: true,
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mkdir_root_is_invalid() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/".into(),
            parents: false,
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mkdir_relative_path_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "Reports".into(),
            parents: false,
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mkdir_brackets_xochitl_when_writing() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/New".into(),
            parents: false,
        },
        false,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    let stop = cmds
        .iter()
        .position(|c| c == "systemctl stop xochitl")
        .expect("stop");
    let start = cmds
        .iter()
        .position(|c| c == "systemctl start xochitl")
        .expect("start");
    assert!(stop < start);
}

#[tokio::test]
async fn mkdir_no_restart_skips_start() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mkdir::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MkdirArgs {
            path: "/Solo".into(),
            parents: false,
        },
        true,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl stop xochitl"));
    assert!(!cmds.iter().any(|c| c == "systemctl start xochitl"));
}

// ---------------------------------------------------------------------------
// mv
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mv_document_to_folder_updates_parent() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uuid.to_string(), DOC_QUICK_UUID);
    assert_eq!(out.from, "/Quick Note");
    assert_eq!(out.to, "/Work");
    assert!(!out.no_op);

    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(
        raw.parent,
        Parent::Folder(uuid::Uuid::parse_str(FOLDER_WORK_UUID).unwrap())
    );
}

#[tokio::test]
async fn mv_to_root_updates_parent_to_empty_string() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Work/Notes".into(),
            dest_folder: "/".into(),
        },
        false,
    )
    .await
    .unwrap();

    let raw = read_metadata(&conn, DOC_NOTES_UUID).await;
    assert_eq!(raw.parent, Parent::Root);
}

#[tokio::test]
async fn mv_out_of_trash_clears_deleted_flag() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/trash/Old Folder".into(),
            dest_folder: "/".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert!(!out.no_op);
    assert_eq!(out.from, "/trash/Old Folder");
    assert_eq!(out.to, "/");

    let raw = read_metadata(&conn, FOLDER_TRASHED_UUID).await;
    assert_eq!(raw.parent, Parent::Root);
    assert!(!raw.deleted);

    let reloaded = build_tree(&conn).await;
    let restored_uuid = uuid::Uuid::parse_str(FOLDER_TRASHED_UUID).unwrap();
    assert_eq!(reloaded.display_path(&restored_uuid), Some("/Old Folder"));
    assert!(!reloaded.get(&restored_uuid).unwrap().is_trashed());
}

#[tokio::test]
async fn mv_same_parent_still_restores_deleted_entries() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let hidden_uuid = "dddddddd-2222-2222-2222-222222222222";
    conn.set_file(
        &format!("{DATA_DIR}/{hidden_uuid}.metadata"),
        br#"{"visibleName":"Hidden Root","type":"DocumentType","parent":"","deleted":true,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
    );
    conn.set_file(
        &format!("{DATA_DIR}/{hidden_uuid}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    let tree = build_tree(&conn).await;

    let out = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Hidden Root".into(),
            dest_folder: "/".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert!(!out.no_op);
    assert_eq!(out.from, "/Hidden Root");
    assert_eq!(out.to, "/");

    let raw = read_metadata(&conn, hidden_uuid).await;
    assert_eq!(raw.parent, Parent::Root);
    assert!(!raw.deleted);
    assert_eq!(raw.version, 2);
}

#[tokio::test]
async fn mv_preserves_unknown_metadata_fields() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Work/Notes".into(),
            dest_folder: "/".into(),
        },
        false,
    )
    .await
    .unwrap();

    let v = read_metadata_value(&conn, DOC_NOTES_UUID).await;
    assert_eq!(v["futureField"]["k"], "v");
    assert_eq!(v["parent"], "");
}

#[tokio::test]
async fn mv_bumps_version() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap();

    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(raw.version, 2);
}

#[tokio::test]
async fn mv_into_self_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Work".into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mv_into_descendant_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Work".into(),
            dest_folder: "/Work/Sub".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mv_to_document_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/Work/Notes".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mv_into_trashed_folder_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: FOLDER_TRASHED_UUID.into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mv_root_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/".into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn mv_same_parent_is_no_op_no_xochitl_calls() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert!(out.no_op);
    let cmds = conn.executed_commands();
    assert!(!cmds.iter().any(|c| c.contains("xochitl")));

    // Version untouched.
    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(raw.version, 1);
}

#[tokio::test]
async fn mv_warns_on_duplicate_name_in_destination() {
    // Add a "Quick Note" inside /Work to set up a collision.
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let collision_uuid = "dddddddd-1111-1111-1111-111111111111";
    conn.set_file(
        &format!("{DATA_DIR}/{collision_uuid}.metadata"),
        format!(
            r#"{{"visibleName":"Quick Note","type":"DocumentType","parent":"{FOLDER_WORK_UUID}","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}}"#
        )
        .as_bytes(),
    );
    conn.set_file(
        &format!("{DATA_DIR}/{collision_uuid}.content"),
        br#"{"fileType":"pdf"}"#,
    );
    let tree = build_tree(&conn).await;

    let out = mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("Quick Note"));
    // Move still happens.
    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(
        raw.parent,
        Parent::Folder(uuid::Uuid::parse_str(FOLDER_WORK_UUID).unwrap())
    );
}

#[tokio::test]
async fn mv_brackets_xochitl_in_order() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    let stop = cmds
        .iter()
        .position(|c| c == "systemctl stop xochitl")
        .expect("stop");
    let start = cmds
        .iter()
        .position(|c| c == "systemctl start xochitl")
        .expect("start");
    assert!(stop < start);
}

#[tokio::test]
async fn mv_no_restart_skips_start() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    mv::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &MvArgs {
            source: "/Quick Note".into(),
            dest_folder: "/Work".into(),
        },
        true,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl stop xochitl"));
    assert!(!cmds.iter().any(|c| c == "systemctl start xochitl"));
}

// ---------------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_document_updates_visible_name() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "Daily Journal".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.uuid.to_string(), DOC_QUICK_UUID);
    assert_eq!(out.old_name, "Quick Note");
    assert_eq!(out.new_name, "Daily Journal");
    assert_eq!(out.path, "/Daily Journal");

    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(raw.visible_name, "Daily Journal");
    assert_eq!(raw.version, 2);
}

#[tokio::test]
async fn rename_folder_updates_visible_name() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Work".into(),
            new_name: "Office".into(),
        },
        false,
    )
    .await
    .unwrap();

    let raw = read_metadata(&conn, FOLDER_WORK_UUID).await;
    assert_eq!(raw.visible_name, "Office");
    assert!(matches!(raw.item_type, ItemType::Collection));

    let reloaded = build_tree(&conn).await;
    assert_eq!(
        reloaded.display_path(&uuid::Uuid::parse_str(FOLDER_WORK_UUID).unwrap()),
        Some("/Office")
    );
}

#[tokio::test]
async fn rename_preserves_unknown_metadata_fields() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Work/Notes".into(),
            new_name: "Notes v2".into(),
        },
        false,
    )
    .await
    .unwrap();

    let v = read_metadata_value(&conn, DOC_NOTES_UUID).await;
    assert_eq!(v["visibleName"], "Notes v2");
    assert_eq!(v["futureField"]["k"], "v");
}

#[tokio::test]
async fn rename_empty_name_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "   ".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn rename_name_with_slash_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "a/b".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn rename_root_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/".into(),
            new_name: "Whatever".into(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn rename_to_existing_sibling_name_warns_but_succeeds() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "duplicate".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("duplicate"));
    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(raw.visible_name, "duplicate");
}

#[tokio::test]
async fn rename_same_name_is_no_op() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "Quick Note".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert!(out.no_op);
    let cmds = conn.executed_commands();
    assert!(!cmds.iter().any(|c| c.contains("xochitl")));
    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(raw.version, 1);
}

#[tokio::test]
async fn rename_trashed_entry_reports_trash_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/trash/Old Folder".into(),
            new_name: "Archive".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.path, "/trash/Archive");
    let raw = read_metadata(&conn, FOLDER_TRASHED_UUID).await;
    assert_eq!(raw.visible_name, "Archive");
}

#[tokio::test]
async fn rename_trashed_entry_no_op_reports_trash_path() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: FOLDER_TRASHED_UUID.into(),
            new_name: "Old Folder".into(),
        },
        false,
    )
    .await
    .unwrap();

    assert!(out.no_op);
    assert_eq!(out.path, "/trash/Old Folder");
}

#[tokio::test]
async fn rename_brackets_xochitl_in_order() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "Daily Notes".into(),
        },
        false,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    let stop = cmds
        .iter()
        .position(|c| c == "systemctl stop xochitl")
        .expect("stop");
    let start = cmds
        .iter()
        .position(|c| c == "systemctl start xochitl")
        .expect("start");
    assert!(stop < start);
}

#[tokio::test]
async fn rename_no_restart_skips_start() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rename::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &RenameArgs {
            path_or_uuid: "/Quick Note".into(),
            new_name: "Daily Notes".into(),
        },
        true,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl stop xochitl"));
    assert!(!cmds.iter().any(|c| c == "systemctl start xochitl"));
}

// ---------------------------------------------------------------------------
// rm
// ---------------------------------------------------------------------------

fn rm_args(paths: &[&str], permanent: bool, recursive: bool) -> RmArgs {
    RmArgs {
        paths: paths.iter().map(|p| (*p).to_string()).collect(),
        permanent,
        recursive,
    }
}

/// Drop the auxiliary files a real notebook UUID accumulates: page `.rm`
/// files, page-level metadata, thumbnails, and a `.pagedata` companion.
/// Lets the permanent-delete tests confirm everything starting with the
/// UUID is wiped (files and dirs alike).
fn add_notebook_pages(conn: &FakeConnection, uuid: &str) {
    conn.set_file(
        &format!("{DATA_DIR}/{uuid}/page-1.rm"),
        b"page-1-stroke-bytes",
    );
    conn.set_file(
        &format!("{DATA_DIR}/{uuid}/page-1-metadata.json"),
        br#"{"layers":[]}"#,
    );
    conn.set_file(&format!("{DATA_DIR}/{uuid}.pagedata"), b"Blank\nGrid\n");
    conn.set_file(
        &format!("{DATA_DIR}/{uuid}.thumbnails/page-1.jpg"),
        b"\xff\xd8\xff\xe0fake-jpeg",
    );
}

#[tokio::test]
async fn rm_soft_delete_document_marks_trashed() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note"], false, false),
        false,
    )
    .await
    .unwrap();

    assert!(!out.permanent);
    assert!(!out.no_op);
    assert_eq!(out.deleted.len(), 1);
    assert_eq!(out.deleted[0].uuid.to_string(), DOC_QUICK_UUID);

    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(raw.parent, Parent::Trash);
    assert!(raw.deleted);
    assert_eq!(raw.version, 2);
}

#[tokio::test]
async fn rm_soft_delete_preserves_unknown_metadata_fields() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work/Notes"], false, false),
        false,
    )
    .await
    .unwrap();

    let v = read_metadata_value(&conn, DOC_NOTES_UUID).await;
    assert_eq!(v["parent"], "trash");
    assert_eq!(v["deleted"], true);
    assert_eq!(v["futureField"]["k"], "v");
}

#[tokio::test]
async fn rm_soft_delete_empty_folder_succeeds() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        // /Work/Sub is empty in the fixture.
        &rm_args(&[FOLDER_SUB_UUID], false, false),
        false,
    )
    .await
    .unwrap();

    let raw = read_metadata(&conn, FOLDER_SUB_UUID).await;
    assert_eq!(raw.parent, Parent::Trash);
    assert!(raw.deleted);
}

#[tokio::test]
async fn rm_soft_delete_non_empty_folder_without_recursive_errors() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work"], false, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));

    // Nothing written.
    let raw = read_metadata(&conn, FOLDER_WORK_UUID).await;
    assert_eq!(raw.parent, Parent::Root);
    assert!(!raw.deleted);
}

#[tokio::test]
async fn rm_soft_delete_non_empty_folder_with_recursive_marks_only_folder() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work"], false, true),
        false,
    )
    .await
    .unwrap();

    let work = read_metadata(&conn, FOLDER_WORK_UUID).await;
    assert_eq!(work.parent, Parent::Trash);
    assert!(work.deleted);

    // Children stay parented to the (now-trashed) folder, not directly trashed.
    let child = read_metadata(&conn, DOC_NOTES_UUID).await;
    assert_eq!(
        child.parent,
        Parent::Folder(uuid::Uuid::parse_str(FOLDER_WORK_UUID).unwrap())
    );
    assert!(!child.deleted);

    // After reload, the child shows up under /trash/Work/...
    let reloaded = build_tree(&conn).await;
    assert_eq!(
        reloaded.display_path(&uuid::Uuid::parse_str(DOC_NOTES_UUID).unwrap()),
        Some("/trash/Work/Notes"),
    );
}

#[tokio::test]
async fn rm_soft_delete_then_mv_out_restores() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note"], false, false),
        false,
    )
    .await
    .unwrap();

    let trashed_tree = build_tree(&conn).await;
    mv::run_with_conn(
        &conn,
        DATA_DIR,
        &trashed_tree,
        &MvArgs {
            source: DOC_QUICK_UUID.into(),
            dest_folder: "/Work".into(),
        },
        false,
    )
    .await
    .unwrap();

    let raw = read_metadata(&conn, DOC_QUICK_UUID).await;
    assert_eq!(
        raw.parent,
        Parent::Folder(uuid::Uuid::parse_str(FOLDER_WORK_UUID).unwrap())
    );
    assert!(!raw.deleted, "mv out of trash should clear deleted flag");
}

#[tokio::test]
async fn rm_idempotent_on_already_trashed_item() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        // The fixture's "Old Folder" already has parent=trash, deleted=true.
        &rm_args(&[FOLDER_TRASHED_UUID], false, false),
        false,
    )
    .await
    .unwrap();

    assert!(out.no_op);
    let cmds = conn.executed_commands();
    assert!(!cmds.iter().any(|c| c.contains("xochitl")));
}

#[tokio::test]
async fn rm_root_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/"], false, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}

#[tokio::test]
async fn rm_multiple_paths_all_processed() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note", "/duplicate"], false, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(
        read_metadata(&conn, DOC_QUICK_UUID).await.parent,
        Parent::Trash
    );
    assert_eq!(
        read_metadata(&conn, DOC_DUP_UUID).await.parent,
        Parent::Trash
    );
}

#[tokio::test]
async fn rm_deduplicates_uuid_and_path_pointing_to_same_item() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note", DOC_QUICK_UUID], false, false),
        false,
    )
    .await
    .unwrap();

    assert_eq!(out.deleted.len(), 1);
}

#[tokio::test]
async fn rm_permanent_document_removes_metadata_content_and_source() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    // /Work/Notes uses a .pdf source — make one so we can assert it's gone.
    conn.set_file(
        &format!("{DATA_DIR}/{DOC_NOTES_UUID}.pdf"),
        b"%PDF-fake-bytes",
    );
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work/Notes"], true, false),
        false,
    )
    .await
    .unwrap();

    for ext in ["metadata", "content", "pdf"] {
        assert!(
            !conn
                .file_exists(&format!("{DATA_DIR}/{DOC_NOTES_UUID}.{ext}"))
                .await
                .unwrap(),
            ".{ext} should be gone",
        );
    }
}

#[tokio::test]
async fn rm_permanent_notebook_removes_pages_and_thumbnails_dirs() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    add_notebook_pages(&conn, DOC_QUICK_UUID);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note"], true, false),
        false,
    )
    .await
    .unwrap();

    for path in [
        format!("{DATA_DIR}/{DOC_QUICK_UUID}.metadata"),
        format!("{DATA_DIR}/{DOC_QUICK_UUID}.content"),
        format!("{DATA_DIR}/{DOC_QUICK_UUID}.pagedata"),
        format!("{DATA_DIR}/{DOC_QUICK_UUID}/page-1.rm"),
        format!("{DATA_DIR}/{DOC_QUICK_UUID}/page-1-metadata.json"),
        format!("{DATA_DIR}/{DOC_QUICK_UUID}.thumbnails/page-1.jpg"),
    ] {
        assert!(
            !conn.file_exists(&path).await.unwrap(),
            "{path} should be gone"
        );
    }
    // Page directories themselves are wiped via remove_dir_all.
    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{DOC_QUICK_UUID}"))
            .await
            .unwrap()
    );
    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{DOC_QUICK_UUID}.thumbnails"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn rm_permanent_folder_recursive_removes_descendants() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let out = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work"], true, true),
        false,
    )
    .await
    .unwrap();

    // Work + descendants Sub + Notes = 3 UUIDs wiped; output `deleted` lists
    // the single explicit target, and descendant_uuids_removed surfaces the rest.
    assert_eq!(out.deleted.len(), 1);
    assert_eq!(out.descendant_uuids_removed, Some(2));

    for uuid in [FOLDER_WORK_UUID, FOLDER_SUB_UUID, DOC_NOTES_UUID] {
        assert!(
            !conn
                .file_exists(&format!("{DATA_DIR}/{uuid}.metadata"))
                .await
                .unwrap(),
            "{uuid}.metadata should be gone"
        );
    }
    // Unrelated items survive.
    assert!(
        conn.file_exists(&format!("{DATA_DIR}/{DOC_QUICK_UUID}.metadata"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn rm_permanent_non_empty_folder_without_recursive_errors() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work"], true, false),
        false,
    )
    .await
    .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
    // Nothing removed.
    assert!(
        conn.file_exists(&format!("{DATA_DIR}/{FOLDER_WORK_UUID}.metadata"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn rm_permanent_preserves_unrelated_uuid_files() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    conn.set_file(&format!("{DATA_DIR}/{DOC_QUICK_UUID}.pdf"), b"%PDF-quick");
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&[DOC_DUP_UUID], true, false),
        false,
    )
    .await
    .unwrap();

    // dup is gone.
    assert!(
        !conn
            .file_exists(&format!("{DATA_DIR}/{DOC_DUP_UUID}.metadata"))
            .await
            .unwrap()
    );
    // Quick Note is untouched.
    assert!(
        conn.file_exists(&format!("{DATA_DIR}/{DOC_QUICK_UUID}.metadata"))
            .await
            .unwrap()
    );
    assert!(
        conn.file_exists(&format!("{DATA_DIR}/{DOC_QUICK_UUID}.pdf"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn rm_permanent_metadata_removed_last() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    conn.set_file(&format!("{DATA_DIR}/{DOC_NOTES_UUID}.pdf"), b"%PDF-notes");
    // Inject a failure on the source-file remove. The metadata remove
    // should never get a chance to run, so the metadata file must survive.
    conn.set_remove_error(&format!("{DATA_DIR}/{DOC_NOTES_UUID}.pdf"), "disk full");
    let tree = build_tree(&conn).await;

    let err = rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Work/Notes"], true, false),
        false,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("disk full"));

    // .metadata still present — item is recoverable in xochitl.
    assert!(
        conn.file_exists(&format!("{DATA_DIR}/{DOC_NOTES_UUID}.metadata"))
            .await
            .unwrap()
    );
    // xochitl was restarted regardless so the tablet isn't left with the
    // service down.
    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl start xochitl"));
}

#[tokio::test]
async fn rm_brackets_xochitl_in_order() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note"], false, false),
        false,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    let stop = cmds
        .iter()
        .position(|c| c == "systemctl stop xochitl")
        .expect("stop");
    let start = cmds
        .iter()
        .position(|c| c == "systemctl start xochitl")
        .expect("start");
    assert!(stop < start);
}

#[tokio::test]
async fn rm_no_restart_skips_start() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    rm::run_with_conn(
        &conn,
        DATA_DIR,
        &tree,
        &rm_args(&["/Quick Note"], false, false),
        true,
    )
    .await
    .unwrap();

    let cmds = conn.executed_commands();
    assert!(cmds.iter().any(|c| c == "systemctl stop xochitl"));
    assert!(!cmds.iter().any(|c| c == "systemctl start xochitl"));
}

#[tokio::test]
async fn rm_empty_paths_rejected() {
    let conn = FakeConnection::new();
    populate(&conn);
    register_xochitl(&conn);
    let tree = build_tree(&conn).await;

    let err = rm::run_with_conn(&conn, DATA_DIR, &tree, &rm_args(&[], false, false), false)
        .await
        .unwrap_err();

    assert!(matches!(downcast_cli(err), CliError::InvalidPath(_)));
}
