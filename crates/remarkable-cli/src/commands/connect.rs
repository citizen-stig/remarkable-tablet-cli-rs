use anyhow::Context;

use crate::commands::common::{self, CommandContext};
use crate::error::CliError;
use crate::output;
use remarkable_tablet::tablet;

/// # Errors
/// Returns an error if the SSH connection fails or device info cannot be fetched.
pub async fn execute(ctx: &CommandContext) -> Result<(), CliError> {
    run(ctx).await.map_err(common::to_cli_error)
}

async fn run(ctx: &CommandContext) -> anyhow::Result<()> {
    let session = ctx.connect().await?;

    ctx.log_verbose("connected; fetching device info");

    let info = tablet::fetch_device_info(&session.ssh, &session.host, ctx.data_dir())
        .await
        .context("fetch device info")?;

    session.ssh.disconnect().await;

    output::print_device_info(&info, ctx.format());
    Ok(())
}
