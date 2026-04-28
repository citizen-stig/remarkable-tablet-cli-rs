# Description

This is standalone cli to interact with remarkable tablet.

# Scope and functionality

Goal of this tool is to let LLM Agents interact with remarkable tablet in a simple way.
It is not MCP server.

## Functionality

It interacts with remarkable tablet via SSH (v2 only, if that matters, since I only have that one)

 - Browse folder and files. get last opened files
 - Backup data to local
 - Sync **new files** from local folder
 - Open documents as PNG so model can do its own text recognition.
 - Claude can organize and clean existing folder structures (move files, create folders, rename folders, delete files, if duplicate found)


It should have:

 * Ergonomic cli interface (clap with clear names, help, descriptions and defaults)
 * Minimal set of dependencies

# Disclaimer

This tool talks to a physical reMarkable tablet over SSH and can read, move,
and delete files on it. **You use it at your own risk.** The authors and
contributors accept no responsibility for data loss, corrupted notebooks, or a
bricked device. Back up anything you care about before running destructive
operations (sync, organize, delete). See the AS-IS and limitation-of-liability
clauses in the license for the full legal position.

# License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
   <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT License ([LICENSE-MIT](LICENSE-MIT) or
   <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.