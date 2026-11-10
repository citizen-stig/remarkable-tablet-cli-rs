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
    pub host_is_explicit: bool,
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

fn workspace_config_path() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .as_deref()
        .and_then(workspace_config_path_from)
}

fn workspace_config_path_from(cwd: &Path) -> Option<PathBuf> {
    let path = cwd
        .join(".config")
        .join("remarkable-cli")
        .join("config.toml");
    path.exists().then_some(path)
}

fn selected_config_path(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(path) => Some(path.to_path_buf()),
        None => workspace_config_path().or_else(default_config_path),
    }
}

/// Local cache directory for paper templates fetched from the tablet:
/// `$XDG_CACHE_HOME/remarkable-cli/templates`, or
/// `$HOME/.cache/remarkable-cli/templates` when `XDG_CACHE_HOME` is unset or
/// not absolute. `None` only when neither variable is available.
#[must_use]
pub fn default_template_cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return Some(xdg.join("remarkable-cli").join("templates"));
        }
    }
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".cache")
            .join("remarkable-cli")
            .join("templates")
    })
}

/// # Errors
/// Returns an error if the config file exists but cannot be read or parsed as TOML.
pub fn load_file_config(path: Option<&Path>) -> anyhow::Result<FileConfig> {
    let Some(p) = selected_config_path(path) else {
        return Ok(FileConfig::default());
    };
    if !p.exists() {
        if path.is_some() {
            anyhow::bail!("config file does not exist: {}", p.display());
        }
        return Ok(FileConfig::default());
    }
    let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    toml::from_str(&raw).with_context(|| format!("parse {}", p.display()))
}

/// Set `host` in the selected config without changing its other keys or comments.
///
/// # Errors
/// Returns an error if no config path is available or the file cannot be read, parsed, or written.
pub(crate) fn set_config_host(path: Option<&Path>, host: &str) -> anyhow::Result<PathBuf> {
    let config_path = selected_config_path(path)
        .ok_or_else(|| anyhow::anyhow!("cannot select config path: HOME is not set"))?;
    let raw = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?
    } else {
        String::new()
    };
    let mut document = raw
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parse {}", config_path.display()))?;
    document["host"] = toml_edit::value(host);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create config directory {}", parent.display()))?;
    }
    std::fs::write(&config_path, document.to_string())
        .with_context(|| format!("write {}", config_path.display()))?;
    Ok(config_path)
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
        host_is_explicit: sources.host.is_explicit(),
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
            config: None,
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
    fn explicit_missing_file_is_an_error() {
        let path = Path::new("/definitely/does/not/exist.toml");
        let err = load_file_config(Some(path)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "config file does not exist: /definitely/does/not/exist.toml"
        );
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

    #[test]
    fn finds_workspace_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(".config")
            .join("remarkable-cli")
            .join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "password = \"from-workspace\"\n").unwrap();

        let selected = workspace_config_path_from(dir.path());
        assert_eq!(selected.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn set_host_preserves_other_config_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# keep this\nhost = \"192.168.1.42\"\npassword = \"secret\"\n",
        )
        .unwrap();

        assert_eq!(set_config_host(Some(&path), "notes.local").unwrap(), path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# keep this\nhost = \"notes.local\"\npassword = \"secret\"\n"
        );
    }
}
