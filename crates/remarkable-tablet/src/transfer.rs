//! Shared remote→local file-transfer primitives used by `backup` and
//! (eventually) reused by `download`'s notebook-page fetch.
//!
//! Two pieces:
//! - [`walk_remote`] enumerates every file under a remote root using
//!   breadth-first traversal with parallel `read_dir` fan-out.
//! - [`download_many`] copies a list of `(remote, local)` jobs to disk,
//!   reusing the same `buffer_unordered`-driven concurrency pattern as
//!   `metadata_loader`.
//!
//! Both buffer entire files in memory via [`TabletConnection::read_file`].
//! That's a known limitation tracked by the TODO on the trait; revisit if
//! a real-world file ever exceeds what's comfortable on RAM.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use futures::stream::{self, StreamExt, TryStreamExt};

use crate::connection::{RemoteFileKind, TabletConnection};
use crate::error::{IoSource, TabletError};

/// Concurrency for SFTP reads when fanning out a directory walk or a
/// batch of file downloads. Matches `metadata_loader::READ_CONCURRENCY`
/// — keeps the SFTP pipeline saturated without overwhelming the tablet.
pub const TRANSFER_CONCURRENCY: usize = 16;

/// One file discovered by [`walk_remote`].
#[derive(Debug, Clone)]
pub struct WalkedFile {
    /// Absolute remote path, suitable for [`TabletConnection::read_file`].
    pub remote_path: String,
    /// Path relative to the walk root, suitable for joining onto a local
    /// destination directory.
    pub rel_path: PathBuf,
    pub size: Option<u64>,
    pub mtime: Option<SystemTime>,
}

/// Recursively enumerate every regular file under `root`. Symlinks and
/// other non-file/non-dir entries are ignored.
///
/// Implementation is iterative (BFS) rather than recursive `async fn` —
/// that avoids `BoxFuture` pinning while still parallelizing the SFTP
/// `read_dir` calls at each depth via `buffer_unordered`.
///
/// # Errors
/// Returns an error if the root cannot be listed, or any nested
/// directory listing fails.
pub async fn walk_remote<C: TabletConnection>(
    conn: &C,
    root: &str,
) -> Result<Vec<WalkedFile>, TabletError> {
    let root = root.trim_end_matches('/').to_string();
    let mut files = Vec::new();
    let mut frontier: Vec<(String, PathBuf)> = vec![(root, PathBuf::new())];

    while !frontier.is_empty() {
        let listings: Vec<(PathBuf, String, Vec<crate::connection::RemoteEntry>)> =
            stream::iter(frontier.drain(..).map(|(remote, rel)| async move {
                let entries = conn.read_dir(&remote).await?;
                Ok::<_, TabletError>((rel, remote, entries))
            }))
            .buffer_unordered(TRANSFER_CONCURRENCY)
            .try_collect()
            .await?;

        for (rel_dir, remote_dir, entries) in listings {
            for entry in entries {
                let child_remote = format!("{remote_dir}/{}", entry.name);
                let child_rel = rel_dir.join(&entry.name);
                match entry.metadata.kind {
                    RemoteFileKind::Dir => {
                        frontier.push((child_remote, child_rel));
                    }
                    RemoteFileKind::File => {
                        files.push(WalkedFile {
                            remote_path: child_remote,
                            rel_path: child_rel,
                            size: entry.metadata.size,
                            mtime: entry.metadata.mtime,
                        });
                    }
                    RemoteFileKind::Other => {
                        // Symlinks etc. — skip silently. The xochitl tree
                        // shouldn't contain these; if it does, surface
                        // them via verbose logging at the caller.
                    }
                }
            }
        }
    }

    Ok(files)
}

/// Copy one remote file to a local path, creating parent directories as
/// needed. Returns the number of bytes written.
///
/// # Errors
/// Returns an error if the remote read fails, parent-dir creation fails,
/// or the local write fails.
pub async fn download_file<C: TabletConnection>(
    conn: &C,
    remote: &str,
    local: impl AsRef<Path>,
) -> Result<u64, TabletError> {
    let bytes = conn.read_file(remote).await?;
    let local = local.as_ref();
    if let Some(parent) = local.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| TabletError::Io {
                op: "create_dir",
                path: parent.display().to_string(),
                source: IoSource::Local(source),
            })?;
    }
    let len = bytes.len() as u64;
    tokio::fs::write(local, &bytes)
        .await
        .map_err(|source| TabletError::Io {
            op: "write",
            path: local.display().to_string(),
            source: IoSource::Local(source),
        })?;
    Ok(len)
}

/// Copy one local file to a remote path. Returns the number of bytes written.
/// The remote's parent directory must already exist (xochitl's `data_dir`
/// always does); this matches `SshConnection::write_file`'s contract.
///
/// # Errors
/// Returns an error if the local read fails or the remote write fails.
pub async fn upload_file<C: TabletConnection>(
    conn: &C,
    local: impl AsRef<Path>,
    remote: &str,
) -> Result<u64, TabletError> {
    let local = local.as_ref();
    let bytes = tokio::fs::read(local)
        .await
        .map_err(|source| TabletError::Io {
            op: "read",
            path: local.display().to_string(),
            source: IoSource::Local(source),
        })?;
    let len = bytes.len() as u64;
    conn.write_file(remote, &bytes).await?;
    Ok(len)
}

/// Copy a batch of files in parallel. Returns total bytes written.
/// Aborts on the first failure (see `try_collect` semantics).
///
/// # Errors
/// Returns an error if any individual download fails.
pub async fn download_many<C: TabletConnection>(
    conn: &C,
    jobs: Vec<(String, PathBuf)>,
) -> Result<u64, TabletError> {
    let totals: Vec<u64> = stream::iter(
        jobs.into_iter()
            .map(|(remote, local)| async move { download_file(conn, &remote, &local).await }),
    )
    .buffer_unordered(TRANSFER_CONCURRENCY)
    .try_collect()
    .await?;
    Ok(totals.into_iter().sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::FakeConnection;

    #[tokio::test]
    async fn walks_flat_root() {
        let conn = FakeConnection::new();
        conn.set_file("/root/a.txt", b"a");
        conn.set_file("/root/b.txt", b"bb");

        let mut files = walk_remote(&conn, "/root").await.unwrap();
        files.sort_by(|x, y| x.rel_path.cmp(&y.rel_path));
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].rel_path, PathBuf::from("a.txt"));
        assert_eq!(files[0].size, Some(1));
        assert_eq!(files[1].rel_path, PathBuf::from("b.txt"));
        assert_eq!(files[1].size, Some(2));
    }

    #[tokio::test]
    async fn walks_nested_dirs() {
        let conn = FakeConnection::new();
        conn.set_file("/root/top.txt", b"x");
        conn.set_file("/root/sub/inner.rm", b"yy");
        conn.set_file("/root/sub/deep/leaf.bin", b"zzz");

        let mut files = walk_remote(&conn, "/root").await.unwrap();
        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        let rels: Vec<_> = files.iter().map(|f| f.rel_path.clone()).collect();
        assert_eq!(
            rels,
            vec![
                PathBuf::from("sub/deep/leaf.bin"),
                PathBuf::from("sub/inner.rm"),
                PathBuf::from("top.txt"),
            ]
        );
        let total: u64 = files.iter().map(|f| f.size.unwrap_or(0)).sum();
        assert_eq!(total, 6);
    }

    #[tokio::test]
    async fn walks_empty_root() {
        let conn = FakeConnection::new();
        conn.mkdir("/root");
        let files = walk_remote(&conn, "/root").await.unwrap();
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn walks_strips_trailing_slash() {
        let conn = FakeConnection::new();
        conn.set_file("/root/a.txt", b"a");
        let files = walk_remote(&conn, "/root/").await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].remote_path, "/root/a.txt");
    }

    #[tokio::test]
    async fn download_file_creates_parent_dirs() {
        let conn = FakeConnection::new();
        conn.set_file("/remote/data.bin", b"hello world");
        let dest_dir = tempfile::tempdir().unwrap();
        let dest = dest_dir.path().join("nested/under/data.bin");

        let written = download_file(&conn, "/remote/data.bin", &dest)
            .await
            .unwrap();
        assert_eq!(written, 11);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
    }

    #[tokio::test]
    async fn download_many_writes_all() {
        let conn = FakeConnection::new();
        conn.set_file("/remote/a", b"aa");
        conn.set_file("/remote/b", b"bbb");
        let dest = tempfile::tempdir().unwrap();
        let jobs = vec![
            ("/remote/a".to_string(), dest.path().join("a")),
            ("/remote/b".to_string(), dest.path().join("b")),
        ];
        let total = download_many(&conn, jobs).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(std::fs::read(dest.path().join("a")).unwrap(), b"aa");
        assert_eq!(std::fs::read(dest.path().join("b")).unwrap(), b"bbb");
    }
}
