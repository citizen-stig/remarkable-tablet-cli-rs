use clap::{Parser, Subcommand, Args, ValueEnum};

use crate::output::OutputFormat;

/// CLI tool for interacting with reMarkable 2 tablet over SSH
#[derive(Debug, Parser)]
#[command(name = "remarkable-cli", version, about)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOptions,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalOptions {
    /// Tablet IP/hostname [auto-discover if omitted]
    #[arg(long, global = true)]
    pub host: Option<String>,

    /// SSH port
    #[arg(long, global = true, default_value_t = 22)]
    pub port: u16,

    /// SSH username
    #[arg(long, global = true, default_value = "root")]
    pub user: String,

    /// SSH password (or set REMARKABLE_PASSWORD env var)
    #[arg(long, global = true, env = "REMARKABLE_PASSWORD")]
    pub password: Option<String>,

    /// SSH private key path
    #[arg(long, global = true, default_value = "~/.ssh/id_rsa")]
    pub key_file: String,

    /// Output format
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// SSH connection timeout in seconds
    #[arg(long, global = true, default_value_t = 5)]
    pub timeout: u64,

    /// Remote xochitl data directory path
    #[arg(long, global = true, default_value = "/home/root/.local/share/remarkable/xochitl")]
    pub data_dir: String,

    /// Skip xochitl restart after mutating operations
    #[arg(long, global = true)]
    pub no_restart: bool,

    /// Enable verbose/debug logging to stderr
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Suppress all stderr output
    #[arg(long, global = true)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Test connectivity and print device info
    Connect,

    /// List folder contents
    Ls(LsArgs),

    /// Show full metadata for a document or folder
    Info(InfoArgs),

    /// Search documents by name pattern
    Find(FindArgs),

    /// Backup tablet data to local directory
    Backup(BackupArgs),

    /// Download a document from the tablet
    Download(DownloadArgs),

    /// Upload PDF or ePub files to tablet
    Upload(UploadArgs),

    /// Move document/folder to a different parent
    Mv(MvArgs),

    /// Create folder(s) on the tablet
    Mkdir(MkdirArgs),

    /// Rename a document or folder
    Rename(RenameArgs),

    /// Delete documents/folders (soft delete by default)
    Rm(RmArgs),
}

// -- Per-command args --

#[derive(Debug, Args)]
pub struct LsArgs {
    /// Path or UUID to list (default: root)
    pub path_or_uuid: Option<String>,

    /// List recursively
    #[arg(short, long)]
    pub recursive: bool,

    /// Maximum depth for recursive listing
    #[arg(long)]
    pub depth: Option<u32>,

    /// Include trashed items
    #[arg(long)]
    pub include_trashed: bool,

    /// Sort order
    #[arg(long, value_enum)]
    pub sort: Option<SortField>,

    /// Display as indented tree (like unix tree command)
    #[arg(long)]
    pub tree: bool,

    /// Show only documents
    #[arg(long, conflicts_with = "folders_only")]
    pub documents_only: bool,

    /// Show only folders
    #[arg(long, conflicts_with = "documents_only")]
    pub folders_only: bool,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Path or UUID of the item
    pub path_or_uuid: String,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    /// Name pattern to search for (substring or glob)
    pub pattern: String,

    /// Filter by item type
    #[arg(long = "type", value_enum)]
    pub item_type: Option<FindTypeFilter>,

    /// Enable case-sensitive matching
    #[arg(long)]
    pub case_sensitive: bool,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Local directory to save backup
    pub local_dir: String,

    /// Only copy files newer than existing backup
    #[arg(long)]
    pub incremental: bool,

    /// Show what would be copied without copying
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct DownloadArgs {
    /// Path or UUID of the document to download
    pub path_or_uuid: String,

    /// Output path (default: ./<visibleName>.<ext>)
    #[arg(long)]
    pub output: Option<String>,

    /// Page range for notebooks (e.g., "1-5" or "1,3,7")
    #[arg(long)]
    pub pages: Option<String>,
}

#[derive(Debug, Args)]
pub struct UploadArgs {
    /// PDF or ePub files to upload
    #[arg(required = true)]
    pub files: Vec<String>,

    /// Parent folder path or UUID (default: root)
    #[arg(long)]
    pub parent: Option<String>,

    /// Custom name for the uploaded document (single file only)
    #[arg(long)]
    pub name: Option<String>,

    /// Show what would be uploaded without uploading
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct MvArgs {
    /// Source document/folder path or UUID
    pub source: String,

    /// Destination folder path or UUID
    pub dest_folder: String,
}

#[derive(Debug, Args)]
pub struct MkdirArgs {
    /// Folder path to create
    pub path: String,

    /// Create intermediate parent folders as needed
    #[arg(short, long)]
    pub parents: bool,
}

#[derive(Debug, Args)]
pub struct RenameArgs {
    /// Path or UUID of the item to rename
    pub path_or_uuid: String,

    /// New name for the item
    pub new_name: String,
}

#[derive(Debug, Args)]
pub struct RmArgs {
    /// Paths or UUIDs of items to delete
    #[arg(required = true)]
    pub paths: Vec<String>,

    /// Permanently delete (skip trash)
    #[arg(long)]
    pub permanent: bool,

    /// Required for deleting non-empty folders
    #[arg(short, long)]
    pub recursive: bool,
}

// -- Helper enums --

#[derive(Debug, Clone, ValueEnum)]
pub enum SortField {
    Name,
    Modified,
    Type,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum FindTypeFilter {
    Document,
    Folder,
    All,
}
