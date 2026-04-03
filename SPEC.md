# Specification: `remarkable-cli` — reMarkable 2 Tablet CLI

## Context

A standalone Rust CLI tool for LLM agents (and humans) to interact with a reMarkable 2 tablet over SSH. Not an MCP server — it's a unix-style CLI with human-readable output by default and `--format json` for agent consumption. The tool covers browsing, backup, upload, .rm rendering to PNG, and file organization.

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
1. Try TCP connect to `10.11.99.1:22` (USB) with 2s timeout
2. Try `remarkable` / `remarkable.local` hostname
3. Read `~/.config/remarkable-cli/config.toml` for saved host
4. Fail with clear error and instructions

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

**Flags**: `--recursive/-r`, `--depth <N>`, `--include-trashed`, `--sort <name|modified|type>`

**Output**: Array of `{ uuid, name, type (folder|document), file_type (pdf|epub|notebook|null), parent_uuid, path, modified, last_opened, tags, pinned, children_count (folders), page_count (docs) }`

### 3.2.1 `remarkable-cli tree [PATH_OR_UUID]`
Print the full folder/document hierarchy as an indented tree (like the unix `tree` command). Defaults to root.

**Flags**: `--depth <N>`, `--include-trashed`, `--sort <name|modified|type>`, `--documents-only`, `--folders-only`

**Human output example**:
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

**JSON output**: nested structure with `{ uuid, name, type, file_type, last_opened, tags, pinned, children: [...] }`

### 3.3 `remarkable-cli info <PATH_OR_UUID>`
Full metadata dump for one item. Returns merged `.metadata` + `.content` with computed fields (full path, file sizes, page UUIDs).

### 3.4 `remarkable-cli recent`
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

### 3.8 `remarkable-cli render <PATH_OR_UUID> [--pages <RANGE>]`
Render .rm notebook pages to PNG.

**Flags**: `--output <DIR>` [default: ./], `--pages <RANGE>` (e.g., `1-5` or `1,3,7`), `--width <PX>` [default: 1404], `--dpi <N>` [default: 226], `--from-backup <DIR>` (render from local backup instead of fetching)

**Behavior**: Fetch .rm files from tablet (or read from backup), parse binary format (v3-v6), rasterize strokes to PNG. Each page is rendered as a **separate PNG file**: `<UUID>_page_<N>.png`.

**Output**: Array of `{ page, output_path, width, height }` — one entry per rendered page.

Note: initial version renders strokes only (no PDF/ePub background compositing — that's a later enhancement).

### 3.9 `remarkable-cli mv <SOURCE> <DEST_FOLDER>`
Move document/folder to a different parent. Updates `parent` in `.metadata`. Stops/restarts xochitl.

### 3.10 `remarkable-cli mkdir <PATH>`
Create folder(s). **Flags**: `--parents/-p` (create intermediate folders).

Generates UUID, writes `.metadata` with `type: CollectionType`. Stops/restarts xochitl.

### 3.11 `remarkable-cli rename <PATH_OR_UUID> <NEW_NAME>`
Update `visibleName` in `.metadata`. Stops/restarts xochitl.

### 3.12 `remarkable-cli rm <PATH_OR_UUID>...`
Delete documents/folders.

**Flags**: `--permanent` (skip trash, delete files), `--recursive/-r` (required for non-empty folders)

Default: soft delete (set `parent: "trash"`, `deleted: true`).

### 3.13 `remarkable-cli purge`
Permanently remove all trashed items. **Flags**: `--yes` (skip confirmation).

### 3.14 `remarkable-cli restart`
Restart xochitl service via SSH. Useful after batching multiple `--no-restart` mutations.

---

## 4. Error Contract

Exit code 0 on success, non-zero on failure. JSON errors:
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

```
src/
  main.rs                 Entry point, clap dispatch
  cli.rs                  Clap derive structs (all commands + global opts)
  config.rs               Config file + env var + CLI flag merging
  connection.rs           SSH/SFTP management, auto-discovery
  tablet.rs               High-level tablet ops (read metadata, stop/restart xochitl)
  metadata.rs             Serde structs for .metadata and .content JSON
  tree.rs                 In-memory document tree from flat metadata
  path_resolver.rs        Human path <-> UUID resolution
  output.rs               JSON / human-readable formatting
  error.rs                Error types and codes
  commands/
    mod.rs, connect.rs, ls.rs, tree.rs, info.rs, recent.rs, find.rs,
    backup.rs, upload.rs, render.rs, mv.rs, mkdir.rs,
    rename.rs, rm.rs, purge.rs, restart.rs
  rm_parser/
    mod.rs                .rm binary parser (v3-v6)
    types.rs              Stroke, Point, Layer, Page types
    render.rs             Rasterize strokes to PNG via image crate
```

### Key dependencies
| Crate                  | Purpose                        |
|------------------------|--------------------------------|
| `clap` (derive)        | CLI parsing                    |
| `ssh2`                 | SSH/SFTP (sync, wraps libssh2) |
| `serde` + `serde_json` | JSON ser/de                    |
| `uuid`                 | UUID v4 generation             |
| `image`                | PNG creation for .rm rendering |
| `toml`                 | Config file parsing            |
| `thiserror`            | Error derivation               |

Sync over async: no need for tokio in a sequential CLI tool. `ssh2` is simpler than `russh`.

Custom .rm parser over external crate: format is small and well-specified, avoids dependency on potentially unmaintained crates.

---

## 7. Task Breakdown

### Phase 1: Foundation
1. **Project scaffolding + CLI skeleton** — Cargo.toml deps, clap derive structs for all commands, main.rs dispatch, output.rs, error.rs. Deliverable: `remarkable-cli --help` works for all subcommands.
2. **SSH connection + auto-discovery** — connection.rs, config.rs, `remarkable-cli connect` command. Deliverable: successful connect + device info.
3. **Metadata parsing + document tree** — metadata.rs serde structs, tablet.rs (read all metadata via SFTP), tree.rs, path_resolver.rs. Deliverable: internal library represents full document tree.
4. **Browse commands** — `ls`, `info`, `recent`, `find`. Deliverable: full read-only browsing.

### Phase 2: Data Transfer
5. **Backup** — SFTP recursive copy, incremental mode, manifest. Deliverable: `remarkable-cli backup ./backups`.
6. **Upload** — UUID generation, metadata creation, file transfer, xochitl restart. Deliverable: `remarkable-cli upload paper.pdf --parent /Research`.

### Phase 3: File Organization
7. **Mutation commands** — `mv`, `mkdir`, `rename` with xochitl stop/restart. Deliverable: reorganize tablet files.
8. **Deletion commands** — `rm` (soft + permanent), `purge`. Deliverable: complete file lifecycle.

### Phase 4: Rendering
9. **.rm binary parser** — Parse v3-v6 format into stroke data. Unit tests with sample files.
10. **PNG rendering** — Rasterize strokes, `remarkable-cli render` command. Deliverable: PNG output of notebook pages.

### Phase 5: Polish
11. **PDF/ePub background compositing** — Render source document pages under stroke layers (optional enhancement).
12. **Testing + documentation + CI** — Integration tests, README, release pipeline.

---

## 8. Verification

For each phase, verify by running the corresponding commands against a real reMarkable 2 tablet:
- **Phase 1**: `remarkable-cli --help`, `remarkable-cli connect`, `remarkable-cli ls`, `remarkable-cli ls --recursive`, `remarkable-cli recent`, `remarkable-cli find "notes"`
- **Phase 2**: `remarkable-cli backup ./test-backup`, `remarkable-cli upload test.pdf --parent /`
- **Phase 3**: `remarkable-cli mkdir /Test`, `remarkable-cli mv <uuid> /Test`, `remarkable-cli rename <uuid> "New Name"`, `remarkable-cli rm <uuid>`
- **Phase 4**: `remarkable-cli render <notebook-uuid> --pages 1 --output ./renders/`
