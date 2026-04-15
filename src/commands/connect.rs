use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::cli::GlobalOptions;
use crate::config::{self, ResolvedConfig};
use crate::connection::{ConnectOptions, SshConnection};
use crate::error::{CliError, Result};
use crate::output;
use crate::tablet;

const USB_HOST: &str = "10.11.99.1";
const USB_PORT: u16 = 22;
const USB_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn execute(global: &GlobalOptions) -> Result<()> {
    run(global).await.map_err(to_cli_error)
}

async fn run(global: &GlobalOptions) -> anyhow::Result<()> {
    let file_cfg = config::load_file_config(None).unwrap_or_default();
    let resolved = config::resolve(global, &file_cfg);
    let host = discover_host(global, &resolved).await?;

    output::log_verbose(global, &format!("connecting to {host}:{}", resolved.port));

    let opts = ConnectOptions {
        user: resolved.user.clone(),
        password: resolved.password.clone(),
        key_file: Some(resolved.key_file.clone()),
        timeout: resolved.timeout,
        verbose: resolved.verbose && !resolved.quiet,
    };

    let ssh = SshConnection::connect(&host, resolved.port, &opts)
        .await
        .context("ssh connect")?;

    output::log_verbose(global, "connected; fetching device info");

    let info = tablet::fetch_device_info(&ssh, &host, &resolved.data_dir)
        .await
        .context("fetch device info")?;

    ssh.disconnect().await;

    output::print_device_info(&info, resolved.format);
    Ok(())
}

async fn discover_host(global: &GlobalOptions, cfg: &ResolvedConfig) -> anyhow::Result<String> {
    if let Some(h) = cfg.host.as_deref() {
        return Ok(h.to_string());
    }
    output::log_verbose(
        global,
        &format!("auto-discover: probing USB fallback {USB_HOST}:{USB_PORT}"),
    );
    let probe = timeout(
        USB_PROBE_TIMEOUT,
        TcpStream::connect(format!("{USB_HOST}:{USB_PORT}")),
    )
    .await;
    match probe {
        Ok(Ok(_)) => Ok(USB_HOST.to_string()),
        _ => Err(anyhow::Error::new(CliError::ConnectionFailed(
            "Could not auto-discover tablet. Connect via USB (10.11.99.1) or pass --host. \
             You can also set the host in ~/.config/remarkable-cli/config.toml."
                .to_string(),
        ))),
    }
}

fn to_cli_error(err: anyhow::Error) -> CliError {
    match err.downcast::<CliError>() {
        Ok(cli) => cli,
        Err(other) => CliError::IoError(format!("{other:#}")),
    }
}
