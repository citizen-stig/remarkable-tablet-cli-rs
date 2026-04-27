use std::fmt;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;

use crate::connection::TabletConnection;
use crate::metadata::{self, DocumentEntry, ItemType};

/// Maximum concurrent SFTP read requests during metadata loading.
///
/// SFTP multiplexes by request ID, so many can be in-flight on a single
/// session. 16 keeps the OpenSSH server's window saturated without
/// overwhelming a USB-attached tablet.
const READ_CONCURRENCY: usize = 16;

/// Tablet IP when reached over the USB Ethernet gadget. Anything else is
/// assumed to be Wi-Fi for `ConnectionType` reporting.
const USB_HOST: &str = "10.11.99.1";

/// How the host was reached. Renders as lowercase `"usb"` / `"wifi"` in both
/// JSON output and human formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    Usb,
    Wifi,
}

impl ConnectionType {
    pub fn for_host(host: &str) -> Self {
        if host == USB_HOST {
            Self::Usb
        } else {
            Self::Wifi
        }
    }
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb => f.write_str("usb"),
            Self::Wifi => f.write_str("wifi"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub host: String,
    pub connection_type: ConnectionType,
    pub firmware_version: String,
    pub battery_percent: u32,
    pub disk_total_mb: u64,
    pub disk_used_mb: u64,
    pub disk_free_mb: u64,
}

pub async fn fetch_device_info<C: TabletConnection>(
    conn: &C,
    host: &str,
    data_dir: &str,
) -> anyhow::Result<DeviceInfo> {
    let (firmware_version, battery_percent, (disk_total_mb, disk_used_mb, disk_free_mb)) = tokio::try_join!(
        fetch_firmware(conn),
        fetch_battery(conn),
        fetch_disk(conn, data_dir),
    )?;
    Ok(DeviceInfo {
        host: host.to_string(),
        connection_type: ConnectionType::for_host(host),
        firmware_version,
        battery_percent,
        disk_total_mb,
        disk_used_mb,
        disk_free_mb,
    })
}

async fn fetch_firmware<C: TabletConnection>(conn: &C) -> anyhow::Result<String> {
    let bytes = conn
        .read_file("/etc/version")
        .await
        .context("read /etc/version")?;
    let s = String::from_utf8(bytes).context("/etc/version not UTF-8")?;
    Ok(s.trim().to_string())
}

async fn fetch_battery<C: TabletConnection>(conn: &C) -> anyhow::Result<u32> {
    let root = "/sys/class/power_supply";
    let entries = conn
        .list_dir(root)
        .await
        .with_context(|| format!("list {root}"))?;
    let mut last_err: Option<anyhow::Error> = None;
    for name in entries {
        let cap_path = format!("{root}/{name}/capacity");
        match conn.read_file(&cap_path).await {
            Ok(raw) => {
                let s = String::from_utf8(raw).context("capacity not UTF-8")?;
                return s
                    .trim()
                    .parse::<u32>()
                    .with_context(|| format!("parse capacity `{}`", s.trim()));
            }
            Err(e) => last_err = Some(e.context(format!("read {cap_path}"))),
        }
    }
    match last_err {
        Some(e) => Err(e.context(format!("no readable battery under {root}"))),
        None => bail!("no battery found under {root}"),
    }
}

async fn fetch_disk<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<(u64, u64, u64)> {
    // df -k emits sizes in 1K blocks. Shell-escape by wrapping in single
    // quotes with any embedded quotes closed-escaped-reopened.
    let escaped = shell_single_quote(data_dir);
    let cmd = format!("df -k {escaped}");
    let out = conn
        .execute(&cmd)
        .await
        .with_context(|| format!("run `{cmd}`"))?;
    let mut lines = out.lines();
    let _header = lines.next();
    // df may wrap long filesystem names onto a second line; concatenate
    // all remaining lines and split on whitespace.
    let rest: String = lines.collect::<Vec<_>>().join(" ");
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 4 {
        return Err(anyhow!("unexpected `df -k` output: {out:?}"));
    }
    // Expected columns: Filesystem 1K-blocks Used Available ...
    // With possible wrapped filesystem, take the last 4+ numeric-looking fields.
    let n = fields.len();
    let total_k: u64 = fields[n - 5]
        .parse()
        .or_else(|_| fields[1].parse())
        .with_context(|| format!("parse total from `df` output: {out:?}"))?;
    let used_k: u64 = fields[n - 4]
        .parse()
        .or_else(|_| fields[2].parse())
        .with_context(|| format!("parse used from `df` output: {out:?}"))?;
    let free_k: u64 = fields[n - 3]
        .parse()
        .or_else(|_| fields[3].parse())
        .with_context(|| format!("parse free from `df` output: {out:?}"))?;
    // 1 KiB → MiB via /1024 (keeps consistency with `df -h`-ish rounding)
    Ok((total_k / 1024, used_k / 1024, free_k / 1024))
}

/// Diagnostic summary of a [`load_all_metadata_full`] run.
#[derive(Debug, Default)]
pub struct LoadDiagnostics {
    /// Total filenames returned by `list_dir(data_dir)`.
    pub dir_entry_count: usize,
    /// Of those, how many matched the `<uuid>.metadata` shape.
    pub uuid_metadata_count: usize,
    /// `(filename, error)` for `.metadata` files that failed to parse.
    pub parse_failures: Vec<(String, String)>,
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
pub async fn load_all_metadata<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<Vec<DocumentEntry>> {
    Ok(load_all_metadata_full(conn, data_dir).await?.0)
}

/// Same as [`load_all_metadata`] but also returns counters and per-file
/// parse errors for diagnostics. Use this when you want to surface why a
/// listing came up empty.
pub async fn load_all_metadata_full<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<(Vec<DocumentEntry>, LoadDiagnostics)> {
    let mut diag = LoadDiagnostics::default();

    let list_start = Instant::now();
    let dir_entries = conn
        .list_dir(data_dir)
        .await
        .with_context(|| format!("list {data_dir}"))?;
    diag.list_dir_elapsed = list_start.elapsed();
    diag.dir_entry_count = dir_entries.len();

    let uuids: Vec<_> = dir_entries
        .iter()
        .filter_map(|name| metadata::extract_uuid(name))
        .collect();
    diag.uuid_metadata_count = uuids.len();

    let read_start = Instant::now();
    // Issue many `read_file` calls concurrently. Each per-uuid future returns
    // either a parsed entry or a parse-failure record; I/O errors bubble out
    // and abort the whole load via `try_collect`.
    let outcomes: Vec<LoadOutcome> = stream::iter(uuids)
        .map(|uuid| load_one(conn, data_dir, uuid))
        .buffer_unordered(READ_CONCURRENCY)
        .try_collect()
        .await?;
    diag.read_elapsed = read_start.elapsed();

    let mut result = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        match outcome {
            LoadOutcome::Entry(e) => result.push(e),
            LoadOutcome::ParseFail(file, err) => diag.parse_failures.push((file, err)),
        }
    }

    Ok((result, diag))
}

enum LoadOutcome {
    Entry(DocumentEntry),
    ParseFail(String, String),
}

async fn load_one<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
    uuid: uuid::Uuid,
) -> anyhow::Result<LoadOutcome> {
    let meta_path = format!("{data_dir}/{uuid}.metadata");
    let content_path = format!("{data_dir}/{uuid}.content");
    // Speculatively fetch `.content` in parallel with `.metadata`. Folders
    // don't have a content file; the read fails and is discarded. Documents
    // (the common case) save a full SFTP round-trip per item.
    let (meta_res, content_res) =
        tokio::join!(conn.read_file(&meta_path), conn.read_file(&content_path),);
    let meta_bytes = meta_res.with_context(|| format!("read {meta_path}"))?;
    let raw = match metadata::parse_metadata(&meta_bytes) {
        Ok(m) => m,
        Err(e) => {
            return Ok(LoadOutcome::ParseFail(
                format!("{uuid}.metadata"),
                e.to_string(),
            ));
        }
    };

    let (file_type, page_count) = if raw.item_type == ItemType::Document {
        match content_res
            .ok()
            .and_then(|b| metadata::parse_content(&b).ok())
        {
            Some(c) => (Some(c.file_type), c.effective_page_count()),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    Ok(LoadOutcome::Entry(DocumentEntry {
        uuid,
        visible_name: raw.visible_name,
        item_type: raw.item_type,
        parent: raw.parent,
        deleted: raw.deleted,
        pinned: raw.pinned,
        last_modified: raw.last_modified,
        version: raw.version,
        tags: raw.tags,
        last_opened: raw.last_opened,
        file_type,
        page_count,
    }))
}

fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::FakeConnection;
    use crate::metadata::{FileType, Parent};
    use uuid::Uuid;

    const DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

    #[tokio::test]
    async fn parses_firmware_and_battery_and_disk() {
        let conn = FakeConnection::new();
        conn.set_file("/etc/version", "20230412102300\n");
        conn.mkdir("/sys/class/power_supply/max77818_battery");
        conn.set_file("/sys/class/power_supply/max77818_battery/capacity", "78\n");
        conn.set_command_output(
            "df -k",
            "Filesystem     1K-blocks    Used Available Use% Mounted on\n/dev/root        6291456 2097152   4194304  33% /\n",
        );
        let info = fetch_device_info(
            &conn,
            "10.11.99.1",
            "/home/root/.local/share/remarkable/xochitl",
        )
        .await
        .unwrap();
        assert_eq!(info.host, "10.11.99.1");
        assert_eq!(info.connection_type, ConnectionType::Usb);
        assert_eq!(info.firmware_version, "20230412102300");
        assert_eq!(info.battery_percent, 78);
        assert_eq!(info.disk_total_mb, 6144);
        assert_eq!(info.disk_used_mb, 2048);
        assert_eq!(info.disk_free_mb, 4096);
    }

    #[tokio::test]
    async fn connection_type_wifi_for_non_usb_ip() {
        let conn = FakeConnection::new();
        conn.set_file("/etc/version", "3.15.4");
        conn.mkdir("/sys/class/power_supply/bat");
        conn.set_file("/sys/class/power_supply/bat/capacity", "50");
        conn.set_command_output(
            "df -k",
            "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/root 1024 256 768 25% /\n",
        );
        let info = fetch_device_info(&conn, "192.168.1.50", "/anywhere")
            .await
            .unwrap();
        assert_eq!(info.connection_type, ConnectionType::Wifi);
    }

    #[test]
    fn shell_quote_basic() {
        assert_eq!(shell_single_quote("abc"), "'abc'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }

    // -- load_all_metadata tests --

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

        let folder = entries.iter().find(|e| e.visible_name == "Work").unwrap();
        assert_eq!(folder.item_type, ItemType::Collection);
        assert_eq!(folder.parent, Parent::Root);
        assert!(folder.file_type.is_none());
        assert_eq!(folder.page_count, None);

        let doc = entries.iter().find(|e| e.visible_name == "Notes").unwrap();
        assert_eq!(doc.item_type, ItemType::Document);
        assert_eq!(
            doc.parent,
            Parent::Folder(Uuid::parse_str(folder_uuid).unwrap())
        );
        assert_eq!(doc.file_type, Some(FileType::Notebook));
        assert_eq!(doc.tags, vec!["work"]);
        assert_eq!(doc.page_count, Some(7));
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
        assert_eq!(entries[0].page_count, Some(4));
    }

    #[tokio::test]
    async fn load_metadata_skips_corrupt() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let good_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let bad_uuid = "bbbbbbbb-cccc-dddd-eeee-ffffffffffff";

        set_metadata(
            &conn,
            good_uuid,
            r#"{"visibleName":"OK","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710000000000,"metadatamodified":1710000000000,"version":1}"#,
        );
        set_metadata(&conn, bad_uuid, "not json at all {{{");

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].visible_name, "OK");
    }

    #[tokio::test]
    async fn load_metadata_fails_on_unreadable_metadata() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let doc_uuid = "11111111-2222-3333-4444-555555555555";
        let folder_uuid = "ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb";

        set_metadata(
            &conn,
            doc_uuid,
            &format!(
                r#"{{"visibleName":"Notes","type":"DocumentType","parent":"{folder_uuid}","deleted":false,"pinned":false,"lastModified":1710604800000,"metadatamodified":1710604800000,"version":1}}"#
            ),
        );
        set_content(&conn, doc_uuid, r#"{"fileType":"notebook"}"#);
        set_metadata(
            &conn,
            folder_uuid,
            r#"{"visibleName":"Work","type":"CollectionType","parent":"","deleted":false,"pinned":false,"lastModified":1710518400000,"metadatamodified":1710518400000,"version":1}"#,
        );

        let folder_meta_path = format!("{DATA_DIR}/{folder_uuid}.metadata");
        conn.set_read_error(&folder_meta_path, "permission denied");

        let err = load_all_metadata(&conn, DATA_DIR).await.unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("read {folder_meta_path}"))
        );
    }

    #[tokio::test]
    async fn load_metadata_missing_content() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        set_metadata(
            &conn,
            uuid,
            r#"{"visibleName":"Doc","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710000000000,"metadatamodified":1710000000000,"version":1}"#,
        );
        // No .content file

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].file_type.is_none());
    }

    #[tokio::test]
    async fn load_metadata_empty_dir() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn load_metadata_ignores_non_metadata_files() {
        let conn = FakeConnection::new();
        conn.mkdir(DATA_DIR);

        let uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        set_metadata(
            &conn,
            uuid,
            r#"{"visibleName":"Doc","type":"DocumentType","parent":"","deleted":false,"pinned":false,"lastModified":1710000000000,"metadatamodified":1710000000000,"version":1}"#,
        );
        // Other files that should be ignored
        conn.set_file(
            &format!("{DATA_DIR}/{uuid}.content"),
            r#"{"fileType":"pdf"}"#,
        );
        conn.set_file(&format!("{DATA_DIR}/{uuid}.pdf"), b"fake pdf");
        conn.set_file(&format!("{DATA_DIR}/random.txt"), "hello");

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_type, Some(FileType::Pdf));
    }
}
