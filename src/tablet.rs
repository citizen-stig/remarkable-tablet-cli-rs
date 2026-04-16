use anyhow::{Context, anyhow, bail};
use serde::Serialize;

use crate::connection::TabletConnection;
use crate::metadata::{self, DocumentEntry, ItemType};

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub host: String,
    pub connection_type: String,
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
    let (firmware_version, battery_percent, (disk_total_mb, disk_used_mb, disk_free_mb)) =
        tokio::try_join!(
            fetch_firmware(conn),
            fetch_battery(conn),
            fetch_disk(conn, data_dir),
        )?;
    Ok(DeviceInfo {
        host: host.to_string(),
        connection_type: connection_type_for(host).to_string(),
        firmware_version,
        battery_percent,
        disk_total_mb,
        disk_used_mb,
        disk_free_mb,
    })
}

pub fn connection_type_for(host: &str) -> &'static str {
    if host == "10.11.99.1" { "usb" } else { "wifi" }
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
    let out = conn.execute(&cmd).await.with_context(|| format!("run `{cmd}`"))?;
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

/// Load all document/folder metadata from the tablet's xochitl data directory.
///
/// Entries that fail to parse are silently skipped so one corrupt file
/// doesn't prevent listing the rest.
pub async fn load_all_metadata<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<Vec<DocumentEntry>> {
    let dir_entries = conn
        .list_dir(data_dir)
        .await
        .with_context(|| format!("list {data_dir}"))?;

    let uuids: Vec<_> = dir_entries
        .iter()
        .filter_map(|name| metadata::extract_uuid(name))
        .collect();

    let mut result = Vec::with_capacity(uuids.len());

    for uuid in uuids {
        let meta_path = format!("{data_dir}/{uuid}.metadata");
        let meta_bytes = match conn.read_file(&meta_path).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        let raw = match metadata::parse_metadata(&meta_bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_type = if raw.item_type == ItemType::Document {
            let content_path = format!("{data_dir}/{uuid}.content");
            match conn.read_file(&content_path).await {
                Ok(b) => metadata::parse_content(&b).ok().map(|c| c.file_type),
                Err(_) => None,
            }
        } else {
            None
        };

        result.push(DocumentEntry {
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
        });
    }

    Ok(result)
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
        conn.set_file(
            "/sys/class/power_supply/max77818_battery/capacity",
            "78\n",
        );
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
        assert_eq!(info.connection_type, "usb");
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
        let info = fetch_device_info(&conn, "192.168.1.50", "/anywhere").await.unwrap();
        assert_eq!(info.connection_type, "wifi");
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
        set_content(&conn, doc_uuid, r#"{"fileType":"notebook"}"#);

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries.len(), 2);

        let folder = entries.iter().find(|e| e.visible_name == "Work").unwrap();
        assert_eq!(folder.item_type, ItemType::Collection);
        assert_eq!(folder.parent, Parent::Root);
        assert!(folder.file_type.is_none());

        let doc = entries.iter().find(|e| e.visible_name == "Notes").unwrap();
        assert_eq!(doc.item_type, ItemType::Document);
        assert_eq!(
            doc.parent,
            Parent::Folder(Uuid::parse_str(folder_uuid).unwrap())
        );
        assert_eq!(doc.file_type, Some(FileType::Notebook));
        assert_eq!(doc.tags, vec!["work"]);
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
        conn.set_file(&format!("{DATA_DIR}/{uuid}.content"), r#"{"fileType":"pdf"}"#);
        conn.set_file(&format!("{DATA_DIR}/{uuid}.pdf"), b"fake pdf");
        conn.set_file(&format!("{DATA_DIR}/random.txt"), "hello");

        let entries = load_all_metadata(&conn, DATA_DIR).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_type, Some(FileType::Pdf));
    }
}
