use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

use crate::cli::{CliValueSource, GlobalOptionSources, GlobalOptions};
use crate::output::OutputFormat;

pub const DEFAULT_PORT: u16 = 22;
pub const DEFAULT_USER: &str = "root";
pub const DEFAULT_KEY_FILE: &str = "~/.ssh/id_rsa";
pub const DEFAULT_DATA_DIR: &str = "/home/root/.local/share/remarkable/xochitl";
pub const DEFAULT_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FileConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub key_file: Option<String>,
    pub format: Option<OutputFormat>,
    pub data_dir: Option<String>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub host: Option<String>,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub key_file: PathBuf,
    pub format: OutputFormat,
    pub data_dir: String,
    pub timeout: Duration,
    pub verbose: bool,
    pub quiet: bool,
}

#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".config")
            .join("remarkable-cli")
            .join("config.toml")
    })
}

/// # Errors
/// Returns an error if the config file exists but cannot be read or parsed as TOML.
pub fn load_file_config(path: Option<&Path>) -> anyhow::Result<FileConfig> {
    let target = match path {
        Some(p) => Some(p.to_path_buf()),
        None => default_config_path(),
    };
    let Some(p) = target else {
        return Ok(FileConfig::default());
    };
    if !p.exists() {
        return Ok(FileConfig::default());
    }
    let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    toml::from_str(&raw).with_context(|| format!("parse {}", p.display()))
}

#[must_use]
pub fn resolve(
    cli: &GlobalOptions,
    sources: &GlobalOptionSources,
    file: &FileConfig,
) -> ResolvedConfig {
    let host = if sources.host.is_explicit() {
        cli.host.clone()
    } else {
        file.host.clone()
    };

    let port = resolve_value(cli.port, sources.port, file.port, DEFAULT_PORT);
    let user = resolve_value(
        cli.user.clone(),
        sources.user,
        file.user.clone(),
        DEFAULT_USER.to_string(),
    );

    let password = if sources.password.is_explicit() {
        cli.password.clone()
    } else {
        file.password.clone()
    };

    let key_file = PathBuf::from(resolve_value(
        cli.key_file.clone(),
        sources.key_file,
        file.key_file.clone(),
        DEFAULT_KEY_FILE.to_string(),
    ));

    let format = resolve_value(cli.format, sources.format, file.format, OutputFormat::Human);

    let data_dir = resolve_value(
        cli.data_dir.clone(),
        sources.data_dir,
        file.data_dir.clone(),
        DEFAULT_DATA_DIR.to_string(),
    );

    let timeout_secs = resolve_value(
        cli.timeout,
        sources.timeout,
        file.timeout,
        DEFAULT_TIMEOUT_SECS,
    );

    ResolvedConfig {
        host,
        port,
        user,
        password,
        key_file,
        format,
        data_dir,
        timeout: Duration::from_secs(timeout_secs),
        verbose: cli.verbose,
        quiet: cli.quiet,
    }
}

fn resolve_value<T: Clone>(
    cli_value: T,
    source: CliValueSource,
    file_value: Option<T>,
    default_value: T,
) -> T {
    if source.is_explicit() {
        cli_value
    } else {
        file_value.unwrap_or(default_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputFormat;

    fn base_cli() -> GlobalOptions {
        GlobalOptions {
            host: None,
            port: DEFAULT_PORT,
            user: DEFAULT_USER.to_string(),
            password: None,
            key_file: DEFAULT_KEY_FILE.to_string(),
            format: OutputFormat::Human,
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
    fn cli_host_beats_file_host() {
        let mut cli = base_cli();
        cli.host = Some("from-cli".into());
        let file = FileConfig {
            host: Some("from-file".into()),
            ..Default::default()
        };
        let r = resolve(
            &cli,
            &GlobalOptionSources {
                host: CliValueSource::CommandLine,
                ..default_sources()
            },
            &file,
        );
        assert_eq!(r.host.as_deref(), Some("from-cli"));
    }

    #[test]
    fn file_host_used_when_cli_missing() {
        let cli = base_cli();
        let file = FileConfig {
            host: Some("from-file".into()),
            ..Default::default()
        };
        let r = resolve(&cli, &default_sources(), &file);
        assert_eq!(r.host.as_deref(), Some("from-file"));
    }

    #[test]
    fn password_from_cli_wins() {
        let mut cli = base_cli();
        cli.password = Some("cli-pw".into());
        let file = FileConfig {
            password: Some("file-pw".into()),
            ..Default::default()
        };
        let r = resolve(
            &cli,
            &GlobalOptionSources {
                password: CliValueSource::CommandLine,
                ..default_sources()
            },
            &file,
        );
        assert_eq!(r.password.as_deref(), Some("cli-pw"));
    }

    #[test]
    fn password_from_file_when_cli_missing() {
        let cli = base_cli();
        let file = FileConfig {
            password: Some("file-pw".into()),
            ..Default::default()
        };
        let r = resolve(&cli, &default_sources(), &file);
        assert_eq!(r.password.as_deref(), Some("file-pw"));
    }

    #[test]
    fn default_data_dir_when_nothing_set() {
        let cli = base_cli();
        let file = FileConfig::default();
        let r = resolve(&cli, &default_sources(), &file);
        assert_eq!(r.data_dir, DEFAULT_DATA_DIR);
    }

    #[test]
    fn file_data_dir_overrides_default() {
        let cli = base_cli();
        let file = FileConfig {
            data_dir: Some("/custom/path".into()),
            ..Default::default()
        };
        let r = resolve(&cli, &default_sources(), &file);
        assert_eq!(r.data_dir, "/custom/path");
    }

    #[test]
    fn cli_port_overrides_file() {
        let mut cli = base_cli();
        cli.port = 2222;
        let file = FileConfig {
            port: Some(3333),
            ..Default::default()
        };
        let r = resolve(
            &cli,
            &GlobalOptionSources {
                port: CliValueSource::CommandLine,
                ..default_sources()
            },
            &file,
        );
        assert_eq!(r.port, 2222);
    }

    #[test]
    fn explicit_default_port_beats_file_value() {
        let cli = base_cli();
        let file = FileConfig {
            port: Some(3333),
            ..Default::default()
        };
        let r = resolve(
            &cli,
            &GlobalOptionSources {
                port: CliValueSource::CommandLine,
                ..default_sources()
            },
            &file,
        );
        assert_eq!(r.port, DEFAULT_PORT);
    }

    #[test]
    fn explicit_default_format_beats_file_value() {
        let cli = base_cli();
        let file = FileConfig {
            format: Some(OutputFormat::Json),
            ..Default::default()
        };
        let r = resolve(
            &cli,
            &GlobalOptionSources {
                format: CliValueSource::CommandLine,
                ..default_sources()
            },
            &file,
        );
        assert_eq!(r.format, OutputFormat::Human);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let cfg = load_file_config(Some(Path::new("/definitely/does/not/exist.toml")))
            .expect("missing file should be OK");
        assert!(cfg.host.is_none());
        assert!(cfg.port.is_none());
    }

    #[test]
    fn load_parses_toml() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, "host = \"192.168.1.10\"\nport = 2222\n").unwrap();
        let cfg = load_file_config(Some(&p)).unwrap();
        assert_eq!(cfg.host.as_deref(), Some("192.168.1.10"));
        assert_eq!(cfg.port, Some(2222));
    }
}
