use std::fmt;

use anyhow::{Context, anyhow, bail};
use serde::Serialize;

use crate::connection::TabletConnection;
use crate::error::CliError;

pub use crate::metadata_loader::{LoadDiagnostics, load_all_metadata, load_all_metadata_full};

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
    #[must_use]
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

/// # Errors
/// Returns an error if any of the firmware, battery, or disk-stat shell calls fails.
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

/// Stop the xochitl document service. Mutating commands must run this before
/// touching `.metadata` / `.content` / source files so xochitl doesn't write
/// over them or read a half-written tree.
///
/// # Errors
/// Returns [`CliError::XochitlError`] if the SSH command fails.
pub async fn stop_xochitl<C: TabletConnection>(conn: &C) -> anyhow::Result<()> {
    conn.execute("systemctl stop xochitl")
        .await
        .map_err(|e| CliError::XochitlError(format!("stop xochitl: {e:#}")))?;
    Ok(())
}

/// Start the xochitl document service after a deliberate [`stop_xochitl`].
/// Pair the two; do not use `systemctl restart` here — that's the deferred
/// `restart` command's job (SPEC §3.15).
///
/// # Errors
/// Returns [`CliError::XochitlError`] if the SSH command fails.
pub async fn start_xochitl<C: TabletConnection>(conn: &C) -> anyhow::Result<()> {
    conn.execute("systemctl start xochitl")
        .await
        .map_err(|e| CliError::XochitlError(format!("start xochitl: {e:#}")))?;
    Ok(())
}

/// Bracket `work` with xochitl stopped, then restarted (skipped under
/// `no_restart`). The restart is attempted even when `work` fails so the
/// tablet doesn't get left with the document service down; `work`'s error
/// takes precedence over a restart failure because it's the user's primary
/// signal.
///
/// # Errors
/// Returns the first failure of: stop, `work`, or restart (in that order
/// of priority).
pub async fn with_xochitl_stopped<C, T, Fut>(
    conn: &C,
    no_restart: bool,
    work: impl FnOnce() -> Fut,
) -> anyhow::Result<T>
where
    C: TabletConnection,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    stop_xochitl(conn).await?;
    let work_result = work().await;
    let restart_result = if no_restart {
        Ok(())
    } else {
        start_xochitl(conn).await
    };
    let value = work_result?;
    restart_result?;
    Ok(value)
}

/// Read `<uuid>.metadata`, mutate selected fields in place via `f`, and
/// write the result back. Round-trips through `serde_json::Value` rather
/// than [`crate::metadata::RawMetadata`] so unknown firmware-specific
/// fields survive — the typed struct silently drops anything it doesn't
/// model.
///
/// # Errors
/// Returns an error if the file cannot be read, is not a JSON object, or
/// the write fails.
pub async fn update_metadata<C: TabletConnection>(
    conn: &C,
    path: &str,
    f: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> anyhow::Result<()> {
    let raw = conn.read_file(path).await?;
    let mut json: serde_json::Value =
        serde_json::from_slice(&raw).with_context(|| format!("parse metadata json: {path}"))?;
    let obj = json
        .as_object_mut()
        .ok_or_else(|| anyhow!("metadata is not a JSON object: {path}"))?;
    f(obj);
    conn.write_file(path, &serde_json::to_vec(&json)?).await?;
    Ok(())
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
        .read_dir(root)
        .await
        .with_context(|| format!("list {root}"))?;
    let mut last_err: Option<anyhow::Error> = None;
    for entry in entries {
        let cap_path = format!("{root}/{}/capacity", entry.name);
        match conn.read_file(&cap_path).await {
            Ok(raw) => {
                let s = String::from_utf8(raw).context("capacity not UTF-8")?;
                return s
                    .trim()
                    .parse::<u32>()
                    .with_context(|| format!("parse capacity `{}`", s.trim()));
            }
            Err(err) => last_err = Some(err.context(format!("read {cap_path}"))),
        }
    }
    match last_err {
        Some(err) => Err(err.context(format!("no readable battery under {root}"))),
        None => bail!("no battery found under {root}"),
    }
}

async fn fetch_disk<C: TabletConnection>(
    conn: &C,
    data_dir: &str,
) -> anyhow::Result<(u64, u64, u64)> {
    let escaped = shell_single_quote(data_dir);
    let cmd = format!("df -k {escaped}");
    let out = conn
        .execute(&cmd)
        .await
        .with_context(|| format!("run `{cmd}`"))?;
    let mut lines = out.lines();
    let _header = lines.next();
    let rest: String = lines.collect::<Vec<_>>().join(" ");
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 4 {
        return Err(anyhow!("unexpected `df -k` output: {out:?}"));
    }

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
    Ok((total_k / 1024, used_k / 1024, free_k / 1024))
}

pub(crate) fn shell_single_quote(s: &str) -> String {
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

    fn register_xochitl(conn: &FakeConnection) {
        conn.set_command_output("systemctl stop xochitl", "");
        conn.set_command_output("systemctl start xochitl", "");
    }

    #[tokio::test]
    async fn with_xochitl_stopped_brackets_work() {
        let conn = FakeConnection::new();
        register_xochitl(&conn);

        let value = with_xochitl_stopped(&conn, false, || async { Ok::<_, anyhow::Error>(42) })
            .await
            .unwrap();
        assert_eq!(value, 42);

        let cmds = conn.executed_commands();
        assert_eq!(
            cmds,
            vec![
                "systemctl stop xochitl".to_string(),
                "systemctl start xochitl".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn with_xochitl_stopped_skips_start_under_no_restart() {
        let conn = FakeConnection::new();
        register_xochitl(&conn);

        with_xochitl_stopped(&conn, true, || async { Ok::<_, anyhow::Error>(()) })
            .await
            .unwrap();

        let cmds = conn.executed_commands();
        assert_eq!(cmds, vec!["systemctl stop xochitl".to_string()]);
    }

    #[tokio::test]
    async fn with_xochitl_stopped_restarts_even_when_work_fails() {
        let conn = FakeConnection::new();
        register_xochitl(&conn);

        let err = with_xochitl_stopped(&conn, false, || async {
            Err::<(), _>(anyhow!("work blew up"))
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("work blew up"));

        let cmds = conn.executed_commands();
        assert!(cmds.iter().any(|c| c == "systemctl start xochitl"));
    }

    #[tokio::test]
    async fn with_xochitl_stopped_prefers_work_error_over_restart_error() {
        let conn = FakeConnection::new();
        conn.set_command_output("systemctl stop xochitl", "");
        // `start` is intentionally not registered → the fake returns an error.

        let err = with_xochitl_stopped(&conn, false, || async {
            Err::<(), _>(anyhow!("primary failure"))
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("primary failure"));
    }

    #[tokio::test]
    async fn update_metadata_preserves_unknown_fields() {
        let conn = FakeConnection::new();
        let path = "/data/abc.metadata";
        conn.set_file(
            path,
            br#"{"visibleName":"Old","type":"DocumentType","parent":"","version":1,"futureField":{"k":"v"}}"#,
        );

        update_metadata(&conn, path, |obj| {
            obj.insert("visibleName".into(), serde_json::json!("New"));
        })
        .await
        .unwrap();

        let raw = conn.read_file(path).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["visibleName"], "New");
        assert_eq!(v["futureField"]["k"], "v");
        assert_eq!(v["version"], 1);
    }

    #[tokio::test]
    async fn update_metadata_rejects_non_object_json() {
        let conn = FakeConnection::new();
        let path = "/data/bad.metadata";
        conn.set_file(path, br#"["not","an","object"]"#);

        let err = update_metadata(&conn, path, |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("not a JSON object"));
    }
}
