use anyhow::Context;

use crate::cli::GlobalOptions;
use crate::commands::common;
use crate::error::Result;
use crate::output;
use crate::tablet;

pub async fn execute(global: &GlobalOptions) -> Result<()> {
    run(global).await.map_err(common::to_cli_error)
}

async fn run(global: &GlobalOptions) -> anyhow::Result<()> {
    let (ssh, cfg) = common::connect(global).await?;

    output::log_verbose(global, "connected; fetching device info");

    let host = cfg
        .host
        .clone()
        .expect("connect succeeded so host must be set");

    let info = tablet::fetch_device_info(&ssh, &host, &cfg.data_dir)
        .await
        .context("fetch device info")?;

    ssh.disconnect().await;

    output::print_device_info(&info, cfg.format);
    Ok(())
}
