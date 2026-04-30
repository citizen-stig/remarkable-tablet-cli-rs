use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use tempfile::TempDir;

use super::{RemoteEntry, RemoteFileKind, RemoteMetadata, TabletConnection};
use crate::error::{IoSource, TabletError};

pub struct FakeConnection {
    root: TempDir,
    commands: std::sync::Mutex<Vec<(String, String)>>,
    read_errors: std::sync::Mutex<HashMap<String, String>>,
    read_dir_errors: std::sync::Mutex<HashMap<String, String>>,
    write_error_suffixes: std::sync::Mutex<Vec<(String, String)>>,
    remove_errors: std::sync::Mutex<HashMap<String, String>>,
    /// Every command that flowed through `execute()`, in call order.
    /// Tests of mutating commands use this to verify xochitl stop/start
    /// is bracketed correctly around writes.
    executed_commands: std::sync::Mutex<Vec<String>>,
}

impl FakeConnection {
    /// # Panics
    /// Panics if creating a temp directory for the fake filesystem fails.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("tempdir"),
            commands: std::sync::Mutex::new(Vec::new()),
            read_errors: std::sync::Mutex::new(HashMap::new()),
            read_dir_errors: std::sync::Mutex::new(HashMap::new()),
            write_error_suffixes: std::sync::Mutex::new(Vec::new()),
            remove_errors: std::sync::Mutex::new(HashMap::new()),
            executed_commands: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn local(&self, remote: &str) -> PathBuf {
        let rel = remote.trim_start_matches('/');
        self.root.path().join(rel)
    }

    /// # Panics
    /// Panics if creating the parent directory or writing the file fails.
    pub fn set_file(&self, path: &str, data: impl AsRef<[u8]>) {
        let p = self.local(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, data.as_ref()).unwrap();
    }

    /// Like [`set_file`](Self::set_file) but also forces the file's mtime.
    /// Tests use this to construct deterministic before/after states for
    /// `--incremental` backup logic, which can't be done with the
    /// near-`now()` mtimes that `std::fs::write` produces.
    ///
    /// # Panics
    /// Panics if creating the parent directory, writing the file, or
    /// applying the mtime fails.
    pub fn set_file_with_mtime(&self, path: &str, data: impl AsRef<[u8]>, mtime: SystemTime) {
        self.set_file(path, data);
        let p = self.local(path);
        filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(mtime))
            .expect("set mtime");
    }

    /// # Panics
    /// Panics if directory creation fails.
    pub fn mkdir(&self, path: &str) {
        std::fs::create_dir_all(self.local(path)).unwrap();
    }

    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn set_command_output(&self, cmd_substring: &str, output: &str) {
        let mut cmds = self.commands.lock().unwrap();
        if let Some(entry) = cmds.iter_mut().find(|(s, _)| s == cmd_substring) {
            entry.1 = output.to_string();
        } else {
            cmds.push((cmd_substring.to_string(), output.to_string()));
        }
    }

    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn set_read_error(&self, path: &str, message: &str) {
        self.read_errors
            .lock()
            .unwrap()
            .insert(path.to_string(), message.to_string());
    }

    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn set_read_dir_error(&self, path: &str, message: &str) {
        self.read_dir_errors
            .lock()
            .unwrap()
            .insert(path.to_string(), message.to_string());
    }

    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn set_write_error_suffix(&self, suffix: &str, message: &str) {
        self.write_error_suffixes
            .lock()
            .unwrap()
            .push((suffix.to_string(), message.to_string()));
    }

    /// Inject a failure for `remove_file(path)`. Used by `rm --permanent`
    /// tests to verify that auxiliary files are removed before metadata so
    /// a partial failure leaves the item visible-but-broken instead of
    /// orphaning its source files.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn set_remove_error(&self, path: &str, message: &str) {
        self.remove_errors
            .lock()
            .unwrap()
            .insert(path.to_string(), message.to_string());
    }

    /// Snapshot of every command passed to `execute()` in call order.
    /// Captured regardless of whether the command had a registered output.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn executed_commands(&self) -> Vec<String> {
        self.executed_commands.lock().unwrap().clone()
    }
}

impl Default for FakeConnection {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrap an injected error message as a synthetic `io::Error` so test failures
/// flow through the same `TabletError::Io` variant as a real local-fs error,
/// rather than introducing a test-only enum variant.
fn injected(message: &str) -> std::io::Error {
    std::io::Error::other(message.to_string())
}

impl TabletConnection for FakeConnection {
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, TabletError> {
        if let Some(message) = self.read_errors.lock().unwrap().get(path).cloned() {
            return Err(TabletError::Io {
                op: "read",
                path: path.to_string(),
                source: IoSource::Local(injected(&format!(
                    "fake injected read_file error: {message}"
                ))),
            });
        }
        let p = self.local(path);
        std::fs::read(&p).map_err(|source| TabletError::Io {
            op: "read",
            path: p.display().to_string(),
            source: IoSource::Local(source),
        })
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), TabletError> {
        if let Some((suffix, message)) = self
            .write_error_suffixes
            .lock()
            .unwrap()
            .iter()
            .find(|(suffix, _)| path.ends_with(suffix.as_str()))
            .cloned()
        {
            return Err(TabletError::Io {
                op: "write",
                path: path.to_string(),
                source: IoSource::Local(injected(&format!(
                    "fake injected write_file error matching suffix {suffix}: {message}"
                ))),
            });
        }
        let p = self.local(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TabletError::Io {
                op: "create_dir",
                path: parent.display().to_string(),
                source: IoSource::Local(source),
            })?;
        }
        std::fs::write(&p, data).map_err(|source| TabletError::Io {
            op: "write",
            path: p.display().to_string(),
            source: IoSource::Local(source),
        })
    }

    async fn read_dir(&self, path: &str) -> Result<Vec<RemoteEntry>, TabletError> {
        if let Some(message) = self.read_dir_errors.lock().unwrap().get(path).cloned() {
            return Err(TabletError::Io {
                op: "read_dir",
                path: path.to_string(),
                source: IoSource::Local(injected(&format!(
                    "fake injected read_dir error: {message}"
                ))),
            });
        }
        let p = self.local(path);
        let entries = std::fs::read_dir(&p).map_err(|source| TabletError::Io {
            op: "read_dir",
            path: p.display().to_string(),
            source: IoSource::Local(source),
        })?;
        let mut out = Vec::new();
        for e in entries {
            let e = e.map_err(|source| TabletError::Io {
                op: "read_dir",
                path: p.display().to_string(),
                source: IoSource::Local(source),
            })?;
            let meta = e.metadata().map_err(|source| TabletError::Io {
                op: "stat",
                path: e.path().display().to_string(),
                source: IoSource::Local(source),
            })?;
            out.push(RemoteEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                metadata: into_remote_metadata(&meta),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn stat(&self, path: &str) -> Result<RemoteMetadata, TabletError> {
        let p = self.local(path);
        let meta = std::fs::metadata(&p).map_err(|source| TabletError::Io {
            op: "stat",
            path: p.display().to_string(),
            source: IoSource::Local(source),
        })?;
        Ok(into_remote_metadata(&meta))
    }

    async fn remove_file(&self, path: &str) -> Result<(), TabletError> {
        if let Some(message) = self.remove_errors.lock().unwrap().get(path).cloned() {
            return Err(TabletError::Io {
                op: "remove_file",
                path: path.to_string(),
                source: IoSource::Local(injected(&format!(
                    "fake injected remove_file error: {message}"
                ))),
            });
        }
        let p = self.local(path);
        std::fs::remove_file(&p).map_err(|source| TabletError::Io {
            op: "remove_file",
            path: p.display().to_string(),
            source: IoSource::Local(source),
        })
    }

    async fn remove_dir_all(&self, path: &str) -> Result<(), TabletError> {
        let p = self.local(path);
        if !p.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&p).map_err(|source| TabletError::Io {
            op: "remove_dir",
            path: p.display().to_string(),
            source: IoSource::Local(source),
        })
    }

    async fn execute(&self, command: &str) -> Result<String, TabletError> {
        self.executed_commands
            .lock()
            .unwrap()
            .push(command.to_string());
        let cmds = self.commands.lock().unwrap();
        for (substr, output) in cmds.iter() {
            if command.contains(substr) {
                return Ok(output.clone());
            }
        }
        // The fake doesn't model an actual shell — an unregistered command is
        // a test-setup omission, surfaced as the dedicated `Mock` variant so
        // the CLI doesn't mis-categorize it as an xochitl service failure.
        Err(TabletError::Mock {
            command: command.to_string(),
        })
    }

    async fn file_exists(&self, path: &str) -> Result<bool, TabletError> {
        Ok(self.local(path).exists())
    }
}

fn into_remote_metadata(meta: &std::fs::Metadata) -> RemoteMetadata {
    let kind = if meta.is_dir() {
        RemoteFileKind::Dir
    } else if meta.is_file() {
        RemoteFileKind::File
    } else {
        RemoteFileKind::Other
    };
    RemoteMetadata {
        size: Some(meta.len()),
        mtime: meta.modified().ok(),
        kind,
    }
}
