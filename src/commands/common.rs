use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::cli::GlobalOptions;
use crate::config::{self, ResolvedConfig};
use crate::connection::{ConnectOptions, SshConnection};
use crate::error::CliError;
use crate::output;
use crate::tablet::{self};
use crate::tree::DocumentTree;

const USB_HOST: &str = "10.11.99.1";
const USB_PORT: u16 = 22;
const USB_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Resolve config, discover the tablet host, and open an SSH session.
///
/// Returns the live `SshConnection` plus the merged config so callers can
/// reuse derived values (data_dir, format, etc.). Caller is responsible for
/// `ssh.disconnect().await` when finished.
pub async fn connect(global: &GlobalOptions) -> anyhow::Result<(SshConnection, ResolvedConfig)> {
    let file_cfg = config::load_file_config(None).unwrap_or_default();
    let mut resolved = config::resolve(global, &file_cfg);
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

    resolved.host = Some(host);
    Ok((ssh, resolved))
}

/// Connect, then load the full document tree from the tablet.
///
/// Convenience for read-only browse commands. Caller is responsible for
/// `ssh.disconnect().await` when finished with the connection.
pub async fn connect_and_load_tree(
    global: &GlobalOptions,
) -> anyhow::Result<(SshConnection, ResolvedConfig, DocumentTree)> {
    let (ssh, cfg) = connect(global).await?;
    output::log_verbose(global, &format!("loading metadata from {}", cfg.data_dir));
    let (entries, diag) = tablet::load_all_metadata_full(&ssh, &cfg.data_dir)
        .await
        .context("load metadata")?;
    output::log_verbose(
        global,
        &format!(
            "xochitl: {} dir entries ({}ms list_dir), {} matched <uuid>.metadata, {} parsed in {}ms, {} parse failures",
            diag.dir_entry_count,
            diag.list_dir_elapsed.as_millis(),
            diag.uuid_metadata_count,
            entries.len(),
            diag.read_elapsed.as_millis(),
            diag.parse_failures.len(),
        ),
    );
    for (file, err) in &diag.parse_failures {
        output::log_verbose(global, &format!("  parse failed: {file}: {err}"));
    }
    Ok((ssh, cfg, DocumentTree::build(entries)))
}

/// Downcast an `anyhow::Error` to `CliError`, falling back to `IoError` so
/// any unstructured failure still produces a usable JSON envelope.
pub fn to_cli_error(err: anyhow::Error) -> CliError {
    match err.downcast::<CliError>() {
        Ok(cli) => cli,
        Err(other) => CliError::IoError(format!("{other:#}")),
    }
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
