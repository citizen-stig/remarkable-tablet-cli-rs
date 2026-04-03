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
 * License permissive for commercial use