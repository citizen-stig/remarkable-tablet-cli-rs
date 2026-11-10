use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use tokio::time::sleep;

use crate::cli::SetupWifiArgs;
use crate::commands::common::{self, CommandContext, MDNS_HOST};
use crate::config;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use remarkable_tablet::connection::TabletConnection;

const NETWORK_DROP_IN_DIR: &str = "/etc/systemd/network/25-wlan0.network.d";
const NETWORK_DROP_IN_PATH: &str =
    "/etc/systemd/network/25-wlan0.network.d/50-remarkable-cli-mdns.conf";
const NETWORK_DROP_IN: &[u8] = b"[Network]\nMulticastDNS=yes\n";
const WIFI_VERIFY_ATTEMPTS: usize = 3;
const WIFI_VERIFY_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Serialize)]
pub struct SetupWifiOutput {
    pub host: String,
    pub wifi_ssh_enabled: bool,
    pub mdns_enabled: bool,
}

/// # Errors
/// Returns an error if SSH, tablet configuration, or Wi-Fi verification fails.
pub async fn execute(ctx: &CommandContext, args: &SetupWifiArgs) -> Result<(), CliError> {
    run(ctx, args).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext, args: &SetupWifiArgs) -> anyhow::Result<()> {
    let connection = ctx.connect().await.context("connect to tablet")?;

    let host = args
        .name
        .as_ref()
        .map_or_else(|| MDNS_HOST.to_string(), |name| format!("{name}.local"));
    let configure_result = configure_wifi(&connection.ssh, args.name.as_deref()).await;
    if let Err(error) = configure_result {
        connection.ssh.disconnect().await;
        return Err(error);
    }

    let wifi_result = connect_wifi(ctx, &host).await;
    connection.ssh.disconnect().await;
    let wifi = wifi_result?;
    wifi.ssh.disconnect().await;

    if args.name.is_some() {
        config::set_config_host(ctx.config_path(), &host).context("save custom mDNS host")?;
    }

    let output = SetupWifiOutput {
        host,
        wifi_ssh_enabled: true,
        mdns_enabled: true,
    };
    println!("{}", format_output(&output, ctx.format()));
    Ok(())
}

async fn connect_wifi(
    ctx: &CommandContext,
    host: &str,
) -> anyhow::Result<common::ConnectedSession> {
    let mut result = ctx.connect_host(host).await;
    for _ in 1..WIFI_VERIFY_ATTEMPTS {
        if result.is_ok() {
            break;
        }
        sleep(WIFI_VERIFY_RETRY_DELAY).await;
        result = ctx.connect_host(host).await;
    }
    result.with_context(|| format!("verify SSH over Wi-Fi through {host}"))
}

async fn configure_wifi<C: TabletConnection>(conn: &C, name: Option<&str>) -> anyhow::Result<()> {
    if let Some(name) = name {
        conn.execute(&format!("hostnamectl set-hostname {name}"))
            .await
            .with_context(|| format!("set tablet hostname to {name}"))?;
    }
    conn.execute("rm-ssh-over-wlan on")
        .await
        .context("enable SSH over Wi-Fi")?;
    conn.execute(&format!("mkdir -p {NETWORK_DROP_IN_DIR}"))
        .await
        .context("create systemd-networkd drop-in directory")?;
    conn.write_file(NETWORK_DROP_IN_PATH, NETWORK_DROP_IN)
        .await
        .with_context(|| format!("write {NETWORK_DROP_IN_PATH}"))?;
    conn.execute("networkctl reload")
        .await
        .context("reload systemd-networkd configuration")?;
    conn.execute("networkctl reconfigure wlan0")
        .await
        .context("reconfigure wlan0")?;

    let wifi_ssh = conn
        .execute("systemctl is-active dropbear-wlan.socket")
        .await
        .context("verify SSH over Wi-Fi")?;
    if wifi_ssh.trim() != "active" {
        anyhow::bail!(
            "SSH over Wi-Fi verification expected active, got {:?}",
            wifi_ssh.trim()
        );
    }

    let mdns = conn
        .execute("resolvectl mdns wlan0")
        .await
        .context("verify mDNS on wlan0")?;
    if !mdns.trim().ends_with(": yes") {
        anyhow::bail!("mDNS verification expected yes, got {:?}", mdns.trim());
    }

    Ok(())
}

fn format_output(output: &SetupWifiOutput, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output::render_json(output),
        OutputFormat::Human => format!(
            "host:             {}\n\
             wifi_ssh_enabled: {}\n\
             mdns_enabled:     {}",
            output.host, output.wifi_ssh_enabled, output.mdns_enabled
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarkable_tablet::connection::FakeConnection;

    fn configured_connection(mdns: &str) -> FakeConnection {
        let conn = FakeConnection::new();
        conn.set_command_output("rm-ssh-over-wlan on", "ssh over WLAN enabled.\n");
        conn.set_command_output("mkdir -p", "");
        conn.set_command_output("networkctl reload", "");
        conn.set_command_output("networkctl reconfigure wlan0", "");
        conn.set_command_output("systemctl is-active dropbear-wlan.socket", "active\n");
        conn.set_command_output("resolvectl mdns wlan0", mdns);
        conn
    }

    #[tokio::test]
    async fn configures_wifi_ssh_and_mdns() {
        let conn = configured_connection("Link 233 (wlan0): yes\n");

        configure_wifi(&conn, None).await.unwrap();

        assert_eq!(
            conn.read_file(NETWORK_DROP_IN_PATH).await.unwrap(),
            NETWORK_DROP_IN
        );
        assert_eq!(
            conn.executed_commands(),
            [
                "rm-ssh-over-wlan on",
                "mkdir -p /etc/systemd/network/25-wlan0.network.d",
                "networkctl reload",
                "networkctl reconfigure wlan0",
                "systemctl is-active dropbear-wlan.socket",
                "resolvectl mdns wlan0",
            ]
        );
    }

    #[tokio::test]
    async fn reports_unexpected_mdns_state() {
        let conn = configured_connection("Link 233 (wlan0): no\n");

        let error = configure_wifi(&conn, None).await.unwrap_err();

        assert_eq!(
            error.to_string(),
            "mDNS verification expected yes, got \"Link 233 (wlan0): no\""
        );
    }

    #[tokio::test]
    async fn reports_unexpected_wifi_ssh_state() {
        let conn = configured_connection("Link 233 (wlan0): yes\n");
        conn.set_command_output("systemctl is-active dropbear-wlan.socket", "inactive\n");

        let error = configure_wifi(&conn, None).await.unwrap_err();

        assert_eq!(
            error.to_string(),
            "SSH over Wi-Fi verification expected active, got \"inactive\""
        );
    }

    #[tokio::test]
    async fn setup_is_idempotent() {
        let conn = configured_connection("Link 233 (wlan0): yes\n");

        configure_wifi(&conn, None).await.unwrap();
        configure_wifi(&conn, None).await.unwrap();

        assert_eq!(
            conn.read_file(NETWORK_DROP_IN_PATH).await.unwrap(),
            NETWORK_DROP_IN
        );
    }

    #[tokio::test]
    async fn custom_name_sets_tablet_hostname() {
        let conn = configured_connection("Link 233 (wlan0): yes\n");
        conn.set_command_output("hostnamectl set-hostname notes", "");

        configure_wifi(&conn, Some("notes")).await.unwrap();

        assert_eq!(
            conn.executed_commands().first().map(String::as_str),
            Some("hostnamectl set-hostname notes")
        );
    }

    #[test]
    fn formats_human_output() {
        let output = SetupWifiOutput {
            host: MDNS_HOST.to_string(),
            wifi_ssh_enabled: true,
            mdns_enabled: true,
        };

        assert_eq!(
            format_output(&output, OutputFormat::Human),
            "host:             remarkable.local\n\
             wifi_ssh_enabled: true\n\
             mdns_enabled:     true"
        );
    }

    #[test]
    fn formats_json_output() {
        let output = SetupWifiOutput {
            host: MDNS_HOST.to_string(),
            wifi_ssh_enabled: true,
            mdns_enabled: true,
        };

        assert_eq!(
            format_output(&output, OutputFormat::Json),
            r#"{"host":"remarkable.local","wifi_ssh_enabled":true,"mdns_enabled":true}"#
        );
    }
}
