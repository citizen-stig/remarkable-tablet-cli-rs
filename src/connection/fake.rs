use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use tempfile::TempDir;

use super::TabletConnection;

pub struct FakeConnection {
    root: TempDir,
    commands: std::sync::Mutex<Vec<(String, String)>>,
    read_errors: std::sync::Mutex<HashMap<String, String>>,
}

impl FakeConnection {
    pub fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("tempdir"),
            commands: std::sync::Mutex::new(Vec::new()),
            read_errors: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn local(&self, remote: &str) -> PathBuf {
        let rel = remote.trim_start_matches('/');
        self.root.path().join(rel)
    }

    pub fn set_file(&self, path: &str, data: impl AsRef<[u8]>) {
        let p = self.local(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, data.as_ref()).unwrap();
    }

    pub fn mkdir(&self, path: &str) {
        std::fs::create_dir_all(self.local(path)).unwrap();
    }

    pub fn set_command_output(&self, cmd_substring: &str, output: &str) {
        let mut cmds = self.commands.lock().unwrap();
        if let Some(entry) = cmds.iter_mut().find(|(s, _)| s == cmd_substring) {
            entry.1 = output.to_string();
        } else {
            cmds.push((cmd_substring.to_string(), output.to_string()));
        }
    }

    pub fn set_read_error(&self, path: &str, message: &str) {
        self.read_errors
            .lock()
            .unwrap()
            .insert(path.to_string(), message.to_string());
    }
}

impl Default for FakeConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl TabletConnection for FakeConnection {
    async fn read_file(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        if let Some(message) = self.read_errors.lock().unwrap().get(path).cloned() {
            return Err(anyhow!(
                "fake injected read_file error for {path}: {message}"
            ));
        }
        let p = self.local(path);
        std::fs::read(&p).with_context(|| format!("fake read_file {}", p.display()))
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> anyhow::Result<()> {
        let p = self.local(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, data).with_context(|| format!("fake write_file {}", p.display()))
    }

    async fn list_dir(&self, path: &str) -> anyhow::Result<Vec<String>> {
        let p = self.local(path);
        let entries =
            std::fs::read_dir(&p).with_context(|| format!("fake list_dir {}", p.display()))?;
        let mut out = Vec::new();
        for e in entries {
            let e = e?;
            out.push(e.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        Ok(out)
    }

    async fn remove_file(&self, path: &str) -> anyhow::Result<()> {
        let p = self.local(path);
        std::fs::remove_file(&p).with_context(|| format!("fake remove_file {}", p.display()))
    }

    async fn execute(&self, command: &str) -> anyhow::Result<String> {
        let cmds = self.commands.lock().unwrap();
        for (substr, output) in cmds.iter() {
            if command.contains(substr) {
                return Ok(output.clone());
            }
        }
        bail!("fake execute: no registered output for command `{command}`")
    }

    async fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        Ok(self.local(path).exists())
    }
}
