use std::path::Path;

use clap::{CommandFactory, FromArgMatches};

use crate::cli::{Cli, CliValueSource, Command, GlobalOptionSources};
use crate::commands::common::CommandContext;
use crate::error::CliError;
use crate::{commands, config, output};

struct ParsedCli {
    cli: Cli,
    sources: GlobalOptionSources,
}

pub async fn run() -> i32 {
    let parsed = ParsedCli::parse();
    let ctx = match build_context(&parsed.cli.global, parsed.sources, None) {
        Ok(ctx) => ctx,
        Err(startup) => {
            output::print_error(&startup.error, startup.format);
            return startup.error.exit_code();
        }
    };

    match dispatch(&ctx, &parsed.cli.command).await {
        Ok(()) => 0,
        Err(error) => {
            output::print_error(&error, ctx.format());
            error.exit_code()
        }
    }
}

async fn dispatch(ctx: &CommandContext, command: &Command) -> Result<(), CliError> {
    match command {
        Command::Connect => commands::connect::execute(ctx).await,
        Command::Ls(args) => commands::ls::execute(ctx, args).await,
        Command::Info(args) => commands::info::execute(ctx, args).await,
        Command::Find(args) => commands::find::execute(ctx, args).await,
        Command::Backup(args) => commands::backup::execute(ctx, args).await,
        Command::Download(args) => commands::download::execute(ctx, args).await,
        Command::Upload(args) => commands::upload::execute(ctx, args).await,
        Command::Mv(args) => commands::mv::execute(ctx, args).await,
        Command::Mkdir(args) => commands::mkdir::execute(ctx, args).await,
        Command::Rename(args) => commands::rename::execute(ctx, args).await,
        Command::Rm(args) => commands::rm::execute(ctx, args).await,
    }
}

fn build_context(
    global: &crate::cli::GlobalOptions,
    sources: GlobalOptionSources,
    config_path: Option<&Path>,
) -> Result<CommandContext, StartupError> {
    let preferred_format = startup_error_format(global.format, sources.format);
    let file_cfg = config::load_file_config(config_path).map_err(|err| StartupError {
        error: crate::error::CliError::IoError(format!("config error: {err:#}")),
        format: preferred_format,
    })?;
    let resolved = config::resolve(global, &sources, &file_cfg);
    Ok(CommandContext::new(global.clone(), resolved))
}

fn startup_error_format(
    cli_format: output::OutputFormat,
    source: CliValueSource,
) -> output::OutputFormat {
    if source.is_explicit() {
        cli_format
    } else {
        output::OutputFormat::Human
    }
}

#[derive(Debug)]
struct StartupError {
    error: crate::error::CliError,
    format: output::OutputFormat,
}

impl ParsedCli {
    fn parse() -> Self {
        let matches = Cli::command().get_matches();
        let cli = Cli::from_arg_matches(&matches).expect("clap validated matches");
        let sources = GlobalOptionSources::from_matches(&matches);
        Self { cli, sources }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::GlobalOptions;
    use crate::config::{
        DEFAULT_DATA_DIR, DEFAULT_KEY_FILE, DEFAULT_PORT, DEFAULT_TIMEOUT_SECS, DEFAULT_USER,
    };

    fn base_global() -> GlobalOptions {
        GlobalOptions {
            host: None,
            port: DEFAULT_PORT,
            user: DEFAULT_USER.to_string(),
            password: None,
            key_file: DEFAULT_KEY_FILE.to_string(),
            format: output::OutputFormat::Human,
            timeout: DEFAULT_TIMEOUT_SECS,
            data_dir: DEFAULT_DATA_DIR.to_string(),
            no_restart: false,
            verbose: false,
            quiet: false,
        }
    }

    fn default_sources() -> GlobalOptionSources {
        GlobalOptionSources {
            host: CliValueSource::Unset,
            ..GlobalOptionSources::default()
        }
    }

    #[test]
    fn config_parse_failure_is_surfaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "format = { definitely = 'not valid toml' }").unwrap();

        let err = build_context(&base_global(), default_sources(), Some(&path)).unwrap_err();
        assert!(matches!(err.error, crate::error::CliError::IoError(_)));
        assert!(err.error.to_string().contains("config error"));
    }

    #[test]
    fn config_format_is_applied_to_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "format = \"json\"\n").unwrap();

        let ctx = build_context(&base_global(), default_sources(), Some(&path)).unwrap();
        assert_eq!(ctx.format(), output::OutputFormat::Json);
    }
}
