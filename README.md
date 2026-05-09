# remarkable-cli

[![CI](https://github.com/nikolaygolub/remarkable-tablet-cli-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/nikolaygolub/remarkable-tablet-cli-rs/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.92-blue)](rust-toolchain.toml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

A command-line tool for browsing, backing up, and organizing files on a
**reMarkable 2** tablet over SSH. Designed for both humans and LLM agents:
human-readable output by default, `--format json` for scripts and agents.

This is **not** an MCP server — it's a unix-style CLI. For the full command
reference, see [`SPEC.md`](SPEC.md).

---

## Disclaimer

This tool talks to a physical reMarkable tablet over SSH and can read, move,
and delete files on it. **You use it at your own risk.** The authors and
contributors accept no responsibility for data loss, corrupted notebooks, or a
bricked device. **Run `backup` before any destructive operation** (`mv`,
`rename`, `rm`, `upload`). See the AS-IS and limitation-of-liability clauses
in the license for the full legal position.

---

## Compatibility

| Aspect           | Supported                                           |
| ---------------- | --------------------------------------------------- |
| Tablet model     | reMarkable 2                                        |
| Firmware         | 3.x (latest tested)                                 |
| Connection       | USB (default) and Wi-Fi                             |
| Host OS — tested | Linux, macOS                                        |
| Host OS — should work | Windows (pure-Rust deps, untested)             |

reMarkable 1 and the Paper Pro are not validated. Older firmware may use
different `xochitl` filesystem layouts and is not supported.

---

## Prerequisites: enable SSH on the tablet

reMarkable 2 ships with SSH built in — no jailbreak required.

1. On the tablet: **Settings → Help → Copyrights and licenses**, scroll to the
   **GPLv3 Compliance** section.
2. Note the **SSH password** and **IP address** shown there.
3. Plug the tablet in via USB (recommended for first-time setup) or connect it
   to the same Wi-Fi as your computer.

The USB connection exposes the tablet at `10.11.99.1` on a virtual network
interface that appears when the cable is plugged in.

---

## Install

### From source (today)

```sh
git clone https://github.com/nikolaygolub/remarkable-tablet-cli-rs
cd remarkable-tablet-cli-rs
cargo install --path crates/remarkable-cli
```

### From git directly

```sh
cargo install --git https://github.com/nikolaygolub/remarkable-tablet-cli-rs remarkable-cli
```

### Prebuilt binaries / `cargo install remarkable-cli`

*Coming soon* — see [project status](#project-status).

---

## Quickstart (5 minutes)

Plug the tablet in via USB, then:

```sh
# 1. Verify connectivity (auto-discovers via USB; prompts for SSH password)
remarkable-cli connect

# 2. List the root folder
remarkable-cli ls

# 3. Back up the entire tablet to a local directory
remarkable-cli backup ./rm-backup
```

That's it — you have a backup. Now you can safely explore the destructive
commands.

---

## Connection & authentication

**Connection** — `--host` is auto-discovered via USB. Override for Wi-Fi:

```sh
remarkable-cli --host 192.168.1.42 ls
```

**Authentication** — tried in this order:

1. SSH agent (whatever is loaded via `ssh-add`).
2. SSH key file (`--key_file`, default `~/.ssh/id_rsa`).
3. Password (`--password`, the `REMARKABLE_PASSWORD` env var, or the config
   file).

Recommended: add your SSH public key to the tablet's `~/.ssh/authorized_keys`
once, and you'll never need the password again.

**Config file** (optional): `~/.config/remarkable-cli/config.toml` can store
defaults for `host`, `user`, `key_file`, etc. Precedence is **CLI flag > env
var > config file > built-in default**.

---

## Common workflows

### 1. Back up before anything destructive

```sh
remarkable-cli backup ./rm-backup
```

Use `--incremental` for subsequent runs to copy only newer files.

### 2. Find a notebook by name

```sh
remarkable-cli find "Meeting Notes"
```

### 3. Download the raw `.rm` page files of a notebook

```sh
remarkable-cli download "/Notebooks/Meeting Notes" --output ./pages/
```

For PDFs and ePubs, the same command fetches the source file directly.

### 4. Render notebook pages to PNG

```sh
remarkable-cli render "/Notebooks/Meeting Notes" --pages 1-5 --output ./renders/
```

Each page is written as a separate PNG: `<UUID>_page_<N>.png`. Stroke layers
only — PDF/ePub backgrounds are not yet composited (planned).

### 5. Upload a PDF into a folder

```sh
remarkable-cli upload paper.pdf --parent /Research
```

The folder must already exist; create it with
`remarkable-cli mkdir --parents /Research` if needed.

---

## Command reference

| Command   | What it does                                                    |
| --------- | --------------------------------------------------------------- |
| `connect` | Test connectivity, print device info (firmware, battery, disk). |
| `ls`      | List a folder. `--tree` for recursive indented output.          |
| `info`    | Full metadata dump for one document or folder.                  |
| `find`    | Search documents by name pattern.                               |
| `backup`  | Recursive SFTP copy of the xochitl data directory.              |
| `download`| Fetch a document's source file or raw `.rm` pages.              |
| `upload`  | Upload PDF or ePub files to the tablet.                         |
| `render`  | Rasterize notebook pages to PNG.                                |
| `mkdir`   | Create a folder. `--parents/-p` for intermediates.              |
| `mv`      | Move a document or folder.                                      |
| `rename`  | Rename a document or folder.                                    |
| `rm`      | Delete (soft by default; `--permanent` skips trash).            |

For full flag documentation, see [`SPEC.md`](SPEC.md) or run
`remarkable-cli <command> --help`.

---

## JSON output for scripting and agents

Every command supports `--format json`. When set, **all** output — including
errors — is JSON, so an agent can parse it uniformly:

```sh
remarkable-cli --format json ls
remarkable-cli --format json info /Notebooks/Meeting\ Notes
```

The error contract (codes like `connection_failed`, `auth_failed`,
`not_found`) is documented in [`SPEC.md`](SPEC.md).

---

## Troubleshooting

| Symptom                        | Likely cause / fix                                                                                                       |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `connection_failed` over USB   | USB cable not connected, or the tablet's USB-network interface is down. Replug.                                          |
| `auth_failed`                  | Password from Settings changed, or `--key_file` doesn't match an entry in `~/.ssh/authorized_keys` on the tablet.        |
| Hostname-based discovery fails | Use the explicit `10.11.99.1` (USB) or the Wi-Fi IP from the tablet's Settings.                                          |
| Notebook renders empty         | Only firmware-3 v6 stroke files are supported; older formats currently fail with `UnsupportedVersion`.                   |

---

## Project status

**Alpha.** The architecture is stable, but several rough edges remain:

- Transfers buffer entire files in memory (no streaming yet).
- No progress indicators on long-running backups.
- SSH host-key verification is permissive (USB-attached tablets reflash their
  host key regularly).
- There is no `restore` command yet — backups are one-way until the next
  release.

See [`PRODUCTION_NOTES_CODEX.md`](PRODUCTION_NOTES_CODEX.md) for the full
production-readiness roadmap and [`CHANGELOG.md`](CHANGELOG.md) for release
history.

---

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test  --workspace --all-features --locked
```

The repo is a Cargo workspace with four crates:

| Crate                | Role                                                            |
| -------------------- | --------------------------------------------------------------- |
| `remarkable-metadata` | Pure-data parsing of `.metadata`/`.content` and document trees.|
| `remarkable-tablet`   | SSH/SFTP client and high-level filesystem operations.          |
| `remarkable-rm`       | Parser and renderer for `.rm` v6 notebook page files.          |
| `remarkable-cli`      | Clap surface and subcommand implementations (the binary).      |

Offline integration tests use a `FakeConnection` backed by a temp directory,
gated behind `remarkable-tablet`'s `test-utils` feature.

---

## License

Licensed under either of:

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License
  ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual-licensed as above, without any additional terms or
conditions.
