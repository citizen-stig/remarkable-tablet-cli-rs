use std::time::{Duration, Instant};

use anyhow::Context;
use futures::stream::{self, StreamExt, TryStreamExt};

use crate::connection::TabletConnection;
use crate::metadata::{self, DocumentEntry};

/// Maximum concurrent SFTP read requests during metadata loading.
///
/// SFTP multiplexes by request ID, so many can be in-flight on a single
/// session. 16 keeps the OpenSSH server's window saturated without
/// overwhelming a USB-attached tablet.
const READ_CONCURRENCY: usize = 16;

/// Diagnostic summary of a [`load_all_metadata_full`] run.
#[derive(Debug, Default)]
pub struct LoadDiagnostics {
    /// Total filenames returned by `list_dir(data_dir)`.
    pub dir_entry_count: usize,
    /// Of those, how many matched the `<uuid>.metadata` shape.
    pub uuid_metadata_count: usize,
    /// `(filename, error)` for `.metadata` files that failed to parse.
    pub parse_failures: Vec<(String, String)>,
    /// `(uuid, error)` for documents whose `.content` could not be read or
    /// parsed. Documents listed here are excluded from the loaded entries:
    /// without a usable `.content` we can't classify their file type.
    pub content_failures: Vec<(uuid::Uuid, String)>,
    /// Wall time spent in the initial `list_dir` SFTP round-trip.
    pub list_dir_elapsed: Duration,
    /// Wall time spent reading + parsing `.metadata` and `.content` files.
    pub read_elapsed: Duration,
}

/// Load all document/folder metadata from the tablet's xochitl data directory.
///
/// Entries that fail to parse are silently skipped so one corrupt file
/// doesn't prevent listing the rest, but metadata read failures abort
/// the load to avoid returning a partial tree.
///
/// # Errors
/// Returns an error if `data_dir` cannot be listed.
pub async fn load_all_metadata<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<Vec<DocumentEntry>> {
    Ok(load_all_metadata_full(conn, data_dir).await?.0)
}

/// Same as [`load_all_metadata`] but also returns counters and per-file
/// parse errors for diagnostics. Use this when you want to surface why a
/// listing came up empty.
///
/// # Errors
/// Returns an error if `data_dir` cannot be listed.
pub async fn load_all_metadata_full<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<(Vec<DocumentEntry>, LoadDiagnostics)> {
    let mut diag = LoadDiagnostics::default();

    let list_start = Instant::now();
    let dir_entries = conn
        .read_dir(data_dir)
        .await
        .with_context(|| format!("list {data_dir}"))?;
    diag.list_dir_elapsed = list_start.elapsed();
    diag.dir_entry_count = dir_entries.len();

    let uuids: Vec<_> = dir_entries
        .iter()
        .filter_map(|entry| metadata::extract_uuid(&entry.name))
        .collect();
    diag.uuid_metadata_count = uuids.len();

    let read_start = Instant::now();
    let outcomes: Vec<LoadOutcome> = stream::iter(uuids)
        .map(|uuid| load_one(conn, data_dir, uuid))
        .buffer_unordered(READ_CONCURRENCY)
        .try_collect()
        .await?;
    diag.read_elapsed = read_start.elapsed();

    let mut result = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        match outcome {
            LoadOutcome::Entry(entry) => result.push(entry),
            LoadOutcome::MetadataParseFail(file, err) => diag.parse_failures.push((file, err)),
            LoadOutcome::ContentReadFail(uuid, err) | LoadOutcome::ContentParseFail(uuid, err) => {
                diag.content_failures.push((uuid, err));
            }
        }
    }

    Ok((result, diag))
}

enum LoadOutcome {
    Entry(DocumentEntry),
    MetadataParseFail(String, String),
    ContentReadFail(uuid::Uuid, String),
    ContentParseFail(uuid::Uuid, String),
}

async fn load_one<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    uuid: uuid::Uuid,
) -> anyhow::Result<LoadOutcome> {
    let meta_path = format!("{data_dir}/{uuid}.metadata");
    let content_path = format!("{data_dir}/{uuid}.content");
    let (meta_res, content_res) =
        tokio::join!(conn.read_file(&meta_path), conn.read_file(&content_path));
    let meta_bytes = meta_res.with_context(|| format!("read {meta_path}"))?;
    let raw = match metadata::parse_metadata(&meta_bytes) {
        Ok(metadata) => metadata,
        Err(err) => {
            return Ok(LoadOutcome::MetadataParseFail(
                format!("{uuid}.metadata"),
                err.to_string(),
            ));
        }
    };

    let content = if raw.item_type == crate::metadata::ItemType::Document {
        let bytes = match content_res {
            Ok(bytes) => bytes,
            Err(err) => return Ok(LoadOutcome::ContentReadFail(uuid, format!("{err:#}"))),
        };
        match metadata::parse_content(&bytes) {
            Ok(content) => Some(content),
            Err(err) => return Ok(LoadOutcome::ContentParseFail(uuid, err.to_string())),
        }
    } else {
        None
    };

    Ok(LoadOutcome::Entry(DocumentEntry::from_raw(
        uuid, raw, content,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::FakeConnection;
    use crate::metadata::{FileType, ItemKind, Parent};
    use uuid::Uuid;

    const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

    fn set_metadata(conn: &FakeConnection, uuid: &str, json: &str) {
        conn.set_file(&format!("{DATA_DIR}/{uuid}.metadata"), json);
    }

    fn set_content(conn: &FakeConnection, uuid: &str, json: &str) {
        conn.set_file(&format!("{DATA_DIR}/{uuid}.content"), json);
    }

    #[tokio::test]
    async fn load_metadata_normal() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let folder_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let doc_uuid = "11111111-2222-3333-4444-555555555555";

        set_metadata(
            &conn,
            folder_uuid,
            r#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
        );
        set_metadata(
            &conn,
            doc_uuid,
            r#"{"visibleName":"Notes","type":"DocumentType","parent":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1,"tags":["work"]}"#,
        );
        set_content(&conn, doc_uuid, r#"{"fileType":"notebook","pageCount":7}"#);

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries.len(), 2);

        let folder = entries
            .iter()
            .find(|entry| entry.visible_name == "Work")
            .unwrap();
        assert!(matches!(folder.kind, ItemKind::Folder));
        assert_eq!(folder.parent, Parent::Root);

        let doc = entries
            .iter()
            .find(|entry| entry.visible_name == "Notes")
            .unwrap();
        assert!(matches!(
            doc.kind,
            ItemKind::Document {
                file_type: FileType::Notebook,
                page_count: Some(7),
            }
        ));
        assert_eq!(
            doc.parent,
            Parent::Folder(Uuid::parse_str(folder_uuid).unwrap())
        );
        assert_eq!(doc.tags, vec!["work"]);
    }

    #[tokio::test]
    async fn load_metadata_missing_content_drops_doc_and_records_diagnostic() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let uuid_str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        set_metadata(
            &conn,
            uuid_str,
            r#"{"visibleName":"Doc","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710000000000,"metadatamodified":1710000000000,"version":1}"#,
        );

        let (entries, diag) = load_all_metadata_full(&conn, DATA_DIR).await.unwrap();
        assert!(entries.is_empty());
        assert_eq!(diag.content_failures.len(), 1);
        assert_eq!(
            diag.content_failures[0].0,
            Uuid::parse_str(uuid_str).unwrap()
        );
    }

    #[tokio::test]
    async fn load_metadata_page_count_from_pages_array() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);
        let doc_uuid = "11111111-2222-3333-4444-555555555555";

        set_metadata(
            &conn,
            doc_uuid,
            r#"{"visibleName":"Old","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}"#,
        );
        set_content(
            &conn,
            doc_uuid,
            r#"{"fileType":"notebook","pages":["a","b","c","d"]}"#,
        );

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries[0].page_count(), Some(4));
    }
}
