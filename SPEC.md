# Specification: `remarkable-cli` — reMarkable 2 Tablet CLI

## Context

A standalone Rust CLI tool for LLM agents (and humans) to interact with a reMarkable 2 tablet over SSH. Not an MCP server — it's a unix-style CLI with human-readable output by default and `--format json` for agent consumption. The tool covers browsing, backup, upload, download, and file organization.

**Supported firmware**: 3.x. Future firmware versions may change the xochitl filesystem layout and require updates.

---

## 1. Binary & Project

- **Binary name**: `remarkable-cli` (via `[[bin]]` in Cargo.toml)
- **Package name**: `remarkable-tablet-cli-rs`
- **License**: MIT or Apache-2.0 (permissive, commercial-friendly)
- **Rust edition**: 2024

---

## 2. Global Options

```
remarkable-cli [GLOBAL OPTIONS] <COMMAND>

  --host <HOST>        Tablet IP/hostname [default: auto-discover]
  --port <PORT>        SSH port [default: 22]
  --user <USER>        SSH username [default: root]
  --password <PASS>    SSH password (or REMARKABLE_PASSWORD env var)
  --key-file <PATH>    SSH private key path [default: ~/.ssh/id_rsa]
  --format <FORMAT>    Output format: json | human [default: human]
  --timeout <SECS>     SSH connection timeout [default: 5]
  --data-dir <PATH>    Remote xochitl path [default: /home/root/.local/share/remarkable/xochitl]
  --no-restart         Skip xochitl restart after mutating operations
  --verbose            Debug logging to stderr
  --quiet              Suppress all stderr output
```

**Auto-discovery** (when `--host` is omitted):
1. Read `~/.config/remarkable-cli/config.toml` for saved host
2. Try TCP connect to `10.11.99.1:22` (USB) with 2s timeout
3. Fail with clear error and instructions

*Note*: hostname-based discovery (`remarkable` / `remarkable.local`) deferred to a later release — config file + USB IP covers the majority of setups.

**Config file** (`~/.config/remarkable-cli/config.toml`): optional, stores defaults for host/user/password/key-file/format. Precedence: CLI flags > env vars > config file > built-in defaults.

**Auth** (tried in order):
1. SSH agent (keys added via `ssh-add`) — no config needed
2. Key-file (`--key-file` or config)
3. Password (`--password`, env var, or config)

---

## 3. Command Reference

### Document addressing convention
All commands that take a document/folder reference accept either:
- **UUID** (e.g., `a1b2c3d4-e5f6-...`) — precise, machine-friendly
- **Human-readable path** (e.g., `/Notebooks/Meeting Notes`) — resolved by walking `parent` chains in metadata

### 3.1 `remarkable-cli connect`
Test connectivity, print device info.

**Output**: `{ host, connection_type (usb|wifi), firmware_version, battery_percent, disk_total_mb, disk_used_mb, disk_free_mb }`

### 3.2 `remarkable-cli ls [PATH_OR_UUID]`
List folder contents (default: root). Shows one level by default.

**Flags**: `--recursive/-r`, `--depth <N>`, `--include-trashed`, `--sort <name|modified|type>`, `--tree` (indented tree output), `--documents-only`, `--folders-only`

**Output**: Array of `{ uuid, name, type (folder|document), file_type (pdf|epub|notebook|null), parent_uuid, path, modified, last_opened, tags, pinned, children_count (folders), page_count (docs) }`

**Tree output** (`ls --tree`): prints the full folder/document hierarchy as an indented tree (like the unix `tree` command).

**Human `--tree` output example**:
```
/
├── Meeting Notes/
│   ├── 2024-03-15.rm          [notebook]  opened: 2024-03-15  tags: work, meetings
│   └── 2024-03-20.rm          [notebook]  opened: 2024-03-20
├── Research/
│   ├── Paper Draft.pdf         [pdf]      opened: 2024-03-18  tags: research
│   └── References/
└── Quick Notes.rm              [notebook]  opened: 2024-03-21
```

**JSON `--tree` output**: nested structure with `{ uuid, name, type, file_type, last_opened, tags, pinned, children: [...] }`

### 3.3 `remarkable-cli info <PATH_OR_UUID>`
Full metadata dump for one item. Returns merged `.metadata` + `.content` with computed fields (full path, file sizes, page UUIDs).

### 3.4 `remarkable-cli recent` *(deferred — sugar for `ls --sort modified`)*
List recently opened/modified documents.

**Flags**: `--count <N>` [default: 10], `--sort <last_opened|last_modified>`

### 3.5 `remarkable-cli find <PATTERN>`
Search by name (substring/glob on `visibleName`).

**Flags**: `--type <document|folder|all>`, `--case-sensitive`

### 3.6 `remarkable-cli backup <LOCAL_DIR>`
Full raw copy of the xochitl data directory via SFTP.

**Flags**: `--incremental` (only newer files), `--dry-run`

**Behavior**: Copies entire xochitl dir to `<LOCAL_DIR>/xochitl/`. Also saves `/etc/version` and device serial. Writes `backup_manifest.json` with timestamp, file count, total size.

### 3.7 `remarkable-cli upload <FILE>...`
Upload PDF or ePub files to the tablet.

**Flags**: `--parent <PATH_OR_UUID>` [default: root], `--name <NAME>` (single file only), `--dry-run`

**Behavior per file**:
1. Validate file is PDF or ePub
2. Generate UUID v4
3. Stop xochitl (once before first file)
4. Create `<UUID>.metadata` (visibleName, type: DocumentType, parent, timestamps)
5. Create `<UUID>.content` (fileType: pdf/epub)
6. SCP the source file as `<UUID>.pdf` / `<UUID>.epub`
7. Restart xochitl after all files (unless `--no-restart`)

Warns on duplicate `visibleName` in target folder.

### 3.8 `remarkable-cli download <PATH_OR_UUID>`
Download a single document's source file (PDF, ePub) or .rm notebook files.

**Flags**: `--output <PATH>` [default: `./<visibleName>.<ext>`], `--pages <RANGE>` (for notebooks: which .rm page files to fetch)

**Behavior**: Fetches the source PDF/ePub file, or for notebooks, the .rm page files into a local directory. Outputs `{ uuid, name, file_type, output_path, size_bytes }`.

### 3.9 `remarkable-cli render <PATH_OR_UUID> [--pages <RANGE>]` *(deferred — Phase 4)*
Render .rm notebook pages to PNG.

**Flags**: `--output <DIR>` [default: ./], `--pages <RANGE>` (e.g., `1-5` or `1,3,7`), `--width <PX>` [default: 1404], `--dpi <N>` [default: 226], `--from-backup <DIR>` (render from local backup instead of fetching)

**Behavior**: Fetch .rm files from tablet (or read from backup), parse binary format (v3-v6), rasterize strokes to PNG. Each page is rendered as a **separate PNG file**: `<UUID>_page_<N>.png`.

**Output**: Array of `{ page, output_path, width, height }` — one entry per rendered page.

Note: initial version renders strokes only (no PDF/ePub background compositing — that's a later enhancement).

### 3.10 `remarkable-cli mv <SOURCE> <DEST_FOLDER>`
Move document/folder to a different parent. Updates `parent` in `.metadata`. Stops/restarts xochitl.

### 3.11 `remarkable-cli mkdir <PATH>`
Create folder(s). **Flags**: `--parents/-p` (create intermediate folders).

Generates UUID, writes `.metadata` with `type: CollectionType`. Stops/restarts xochitl.

### 3.12 `remarkable-cli rename <PATH_OR_UUID> <NEW_NAME>`
Update `visibleName` in `.metadata`. Stops/restarts xochitl.

### 3.13 `remarkable-cli rm <PATH_OR_UUID>...`
Delete documents/folders.

**Flags**: `--permanent` (skip trash, delete files), `--recursive/-r` (required for non-empty folders)

Default: soft delete (set `parent: "trash"`, `deleted: true`).

### 3.14 `remarkable-cli purge` *(deferred)*
Permanently remove all trashed items. **Flags**: `--yes` (skip confirmation).

### 3.15 `remarkable-cli restart` *(deferred)*
Restart xochitl service via SSH. Useful after batching multiple `--no-restart` mutations.

---

## 4. Error Contract

Exit code 0 on success, non-zero on failure. When `--format json` is set, **all** output (success and errors) is JSON — LLM agents can parse everything uniformly.

JSON error format:
```json
{ "error": true, "code": "connection_failed", "message": "..." }
```
Error codes: `connection_failed`, `auth_failed`, `not_found`, `already_exists`, `invalid_path`, `permission_denied`, `xochitl_error`, `format_error`, `io_error`.

---

## 5. reMarkable 2 Filesystem Reference

- **Data path**: `/home/root/.local/share/remarkable/xochitl/`
- **Flat structure**: all items stored as UUID-named files at the top level
- **Per-item files**: `<UUID>.metadata` (JSON), `<UUID>.content` (JSON), `<UUID>/` dir with `.rm` pages, plus source PDF/ePub
- **`.metadata` fields**: `visibleName`, `type` (CollectionType | DocumentType), `parent` (UUID, "" = root, "trash" = deleted), `deleted`, `pinned`, `lastModified` (epoch ms), `metadatamodified`, `version`
- **Hierarchy**: logical via `parent` references, not filesystem directories
- **xochitl**: must be stopped before modifications, restarted after (`systemctl stop/restart xochitl`)

---

## 6. Architecture

The crate has both a library and a binary target. `src/lib.rs` re-exports every module so `tests/` (integration) can reach `FakeConnection`, `fetch_device_info`, etc. directly. `src/main.rs` is a thin wrapper that calls into the library.

```
src/
  lib.rs                  Module declarations (pub) for the library target
  main.rs                 Entry point; `#[tokio::main] async fn main`, clap dispatch
  cli.rs                  Clap derive structs (all commands + global opts)
  config.rs               Config file + env var + CLI flag merging
  connection.rs           TabletConnection trait + SshConnection + FakeConnection
  tablet.rs               High-level tablet ops (device info, metadata reads, xochitl stop/restart)
  metadata.rs             Serde structs for .metadata and .content JSON
  tree.rs                 In-memory document tree from flat metadata
  path_resolver.rs        Human path <-> UUID resolution
  output.rs               JSON / human-readable formatting; verbose-log helper
  error.rs                Error types and codes
  commands/
    mod.rs, connect.rs, ls.rs, info.rs, find.rs,
    backup.rs, download.rs, upload.rs, mv.rs, mkdir.rs,
    rename.rs, rm.rs
  rm_parser/              (Phase 4 — deferred)
    mod.rs                .rm binary parser (v3-v6)
    types.rs              Stroke, Point, Layer, Page types
    render.rs             Rasterize strokes to PNG via image crate
tests/
  connect.rs              Integration test for `connect` via FakeConnection
  fixtures/               Sample .metadata and .content JSON files
```

### Testability: `TabletConnection` trait

The core abstraction for testability. Defined in `connection.rs` using native `async fn` in traits (edition 2024):

```rust
pub trait TabletConnection {
    async fn read_file(&self, path: &str) -> anyhow::Result<Vec<u8>>;
    async fn write_file(&self, path: &str, data: &[u8]) -> anyhow::Result<()>;
    async fn list_dir(&self, path: &str) -> anyhow::Result<Vec<String>>;
    async fn remove_file(&self, path: &str) -> anyhow::Result<()>;
    async fn execute(&self, command: &str) -> anyhow::Result<String>;
    async fn file_exists(&self, path: &str) -> anyhow::Result<bool>;
}
```

Path arguments are `&str` because they represent remote SFTP paths, which are UTF-8 strings per RFC 4251 §5 and draft-ietf-secsh-filexfer — not local OS-native paths. `std::path::Path` would be semantically wrong here (it carries OS-specific encoding and separator rules that don't apply to remote paths). No explicit `Send` bounds on the returned futures — nothing in the crate spawns tasks, so `Send` is not required. If `tokio::spawn` is introduced later, add bounds at that point. Consumers take `&C` with `C: TabletConnection` (generic monomorphization) rather than `&dyn TabletConnection`.

- **Production**: `SshConnection` implements the trait via `russh`/`russh-sftp` (TCP probe → SSH handshake → SFTP subsystem). Auth tries SSH agent → key-file → password in order. Host-key verification accepts any server key — USB-connected tablets reflash their host key regularly; strict checking is impractical.
- **Tests**: `FakeConnection` (also in `connection.rs`, exported publicly) operates on a `tempfile::TempDir`, enabling offline unit and integration tests. `set_file`, `mkdir`, and `set_command_output` let a test populate remote state; `execute` looks up registered substrings in the command.

### Key dependencies
| Crate                                       | Purpose                                                                 |
|---------------------------------------------|-------------------------------------------------------------------------|
| `clap` (derive + env)                       | CLI parsing, env-var sourcing for `REMARKABLE_PASSWORD`                 |
| `russh` (pinned `=0.57.1`) + `russh-sftp`   | SSH/SFTP (pure Rust, no native C deps)                                  |
| `tokio` (rt-multi-thread, macros, net, time, io-util, fs, sync) | Async runtime (required by russh)                    |
| `serde` + `serde_json`                      | JSON ser/de                                                             |
| `uuid`                                      | UUID v4 generation                                                      |
| `image`                                     | PNG creation for .rm rendering (Phase 4)                                |
| `toml`                                      | Config file parsing                                                     |
| `anyhow`                                    | Application error propagation                                           |
| `thiserror`                                 | Structured error types (JSON error codes)                               |
| `tempfile`                                  | Backing store for `FakeConnection` (declared as a regular dep, not dev) |

Why `russh` over `ssh2`: `ssh2` wraps libssh2 (C), which requires OpenSSL/libssh2 system libraries and causes build issues across platforms and CI. `russh` is pure Rust — compiles everywhere with zero native deps. The async requirement is minimal (`#[tokio::main]` on main).

Why `russh` is pinned to `=0.57.1`: as of April 2026 the latest `russh` (0.60.0) pulls in `internal-russh-forked-ssh-key` → `p256 0.14.0-rc.8`, which fails to compile against the rc.30+ series of `elliptic-curve` / `primeorder`. Pin back to 0.57.1 until the rc series stabilizes.

Why `anyhow` + `thiserror`: internal fallible code returns `anyhow::Result<T>`. Errors that need specific JSON codes are attached via `anyhow::Error::new(CliError::…)`; the top-level `execute` in each command downcasts at the output boundary. Generic errors fall through to `CliError::IoError`. This keeps propagation ergonomic while preserving the structured error contract for agent consumers.

No `async-trait`: edition 2024 supports native `async fn` in traits. The trait uses plain `async fn` without explicit `Send` bounds — nothing in the crate spawns tasks, so `Send` is unnecessary.

Custom .rm parser over external crate: format is small and well-specified, avoids dependency on potentially unmaintained crates.

---

## 7. Task Breakdown

### +Phase 1: Foundation
1. + **Project scaffolding + CLI skeleton** — Cargo.toml deps, clap derive structs for all commands, main.rs dispatch, output.rs, error.rs. Deliverable: `remarkable-cli --help` works for all subcommands.
2. + **`TabletConnection` trait + SSH implementation + config** — connection.rs (trait + `SshConnection`), config.rs, `FakeConnection` for tests. Deliverable: `remarkable-cli connect` works.
3. + **Metadata parsing + document tree** — metadata.rs serde structs, tablet.rs (read all metadata via trait), tree.rs, path_resolver.rs + unit tests using fixtures. Deliverable: internal library represents full document tree.
4. + **Browse commands** — `ls` (with `--tree`/`--recursive`), `info`, `find`. Deliverable: full read-only browsing with tests.

### +Phase 2: Data Transfer
5. +**Backup** — SFTP recursive copy, incremental mode, manifest. Deliverable: `remarkable-cli backup ./backups`.
6. +**Download** — Single document fetch. Deliverable: `remarkable-cli download <uuid> --output ./doc.pdf`.
7. +**Upload** — UUID generation, metadata creation, file transfer, xochitl restart. Deliverable: `remarkable-cli upload paper.pdf --parent /Research`.

### +Phase 3: File Organization
8. +**Mutation commands** — `mv`, `mkdir`, `rename` with xochitl stop/restart. Deliverable: reorganize tablet files.
9. +**Deletion commands** — `rm` (soft + permanent). Deliverable: complete file lifecycle.

### Phase 4: Rendering *(deferred — not in MVP)*
+Preparation:
 - +split into workspace with crates:
    - +`remarkable-metadata`: pure-data parsing of `.metadata`/`.content`, document tree, path resolver (publishable)
    - +`remarkable-tablet`: SSH/SFTP client, file ops, xochitl control; depends on `remarkable-metadata`
    - +`remarkable-rm`: stub for the `.rm` binary format work below (publishable)
    - +`remarkable-cli`: clap surface and commands; depends on the three above
    - +`FakeConnection` lives behind a `test-utils` feature on `remarkable-tablet`, so `tempfile`/`filetime` are no longer runtime deps of the CLI
10. +**.rm v6 binary parser** — Parse v6 stroke data into a `Page { layers, text, paper_size }`. Slice-based `Reader<'a>` with bounded sub-readers; type-system rejects invalid pen/color/tag values. Four real-tablet v6 fixtures (`smoke`, `pens-small`, `edits`, `layers`) drive integration tests; ground truth in `crates/remarkable-rm/tests/fixtures/README.md`.
   - +sample files: 4 v6 captures from a reMarkable 2 (firmware 3.x)
   - +rely on type-system for robust implementation: closed enums + bounded sub-readers
   - **Deferred to a follow-up**:
     - **v3 / v5 parsing** — separate code path, no fixtures yet; entry point currently errors with `UnsupportedVersion(n)` for non-6 headers.
     - **Surfacing GlyphRange / PageInfo / AuthorIds / SceneTextItem / Tombstone** — bytes are consumed correctly, but no public API exposes them (renderer doesn't need them).
     - **Inline / paragraph text formatting** — `RootText` parser handles the wire format, but none of the current fixtures contain text blocks, so the path is unexercised.
     - **Anchor-group fixture confirmation** (spec §5.3 TODO) — only the new LWW-wrapped layout (indices 7-10) is implemented; the older 4-6 layout has no fixture coverage.
     - **Round-trip / writer support** — read-only renderer follows ddvk and discards trailing `extra_data`. Round-trip would need rmscene-style preservation.
11. **PNG rendering** — Rasterize strokes, `remarkable-cli render` command. Deliverable: PNG output of notebook pages.

### Phase 5: Polish
12. **Deferred commands** — `recent`, `purge`, `restart`, hostname auto-discovery.
13. **PDF/ePub background compositing** — Render source document pages under stroke layers (optional enhancement).
14. **CI + release pipeline** — GitHub Actions, cross-compilation, README.
15. Publishing:
    - cli: installing via  cargo install and/or archlinux AUR
    - binary parser on crates.io
    - metadata reader on crates.io

### Phase 6: Discovered During implementation

 - tag manipulation: add/remove
 - filter by tag
 - count by tag:
   - "read" command: download/render in the same step, so it so consumable by LLM agent
   - download destination folder

---

## 8. Testing Strategy

### Unit tests (offline, run in CI)
- **Metadata parsing**: deserialize sample `.metadata` / `.content` JSON from `tests/fixtures/`
- **Document tree building**: construct tree from fixture metadata, verify parent/child relationships
- **Path resolution**: resolve human paths to UUIDs and back using fixture trees
- **Config merging**: verify precedence (CLI > env > config file > defaults)
- **Output formatting**: verify JSON and human-readable output for known inputs

### Integration tests (offline, run in CI)
- Use `FakeConnection` (implements `TabletConnection` with a temp directory)
- Populate temp dir with fixture metadata files
- Run commands end-to-end and assert output
- Test mutating commands (`mv`, `rename`, `rm`, `upload`) verify filesystem changes

### Acceptance tests (manual, requires real tablet)
- **Phase 1**: `remarkable-cli --help`, `remarkable-cli connect`, `remarkable-cli ls`, `remarkable-cli ls --tree`, `remarkable-cli find "notes"`
- **Phase 2**: `remarkable-cli backup ./test-backup`, `remarkable-cli download <uuid>`, `remarkable-cli upload test.pdf --parent /`
- **Phase 3**: `remarkable-cli mkdir /Test`, `remarkable-cli mv <uuid> /Test`, `remarkable-cli rename <uuid> "New Name"`, `remarkable-cli rm <uuid>`
- **Phase 4**: `remarkable-cli render <notebook-uuid> --pages 1 --output ./renders/`
