use std::path::PathBuf;

use clap::{ArgMatches, Args, Parser, Subcommand, ValueEnum, parser::ValueSource};

use crate::output::OutputFormat;
use remarkable_metadata::SortField;
use remarkable_metadata::page_range::PageSelection;
use remarkable_metadata::tree::EntryKindFilter;

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

#[derive(Debug, Clone, Args)]
pub struct GlobalOptions {
    /// Tablet IP/hostname [auto-discover if omitted]
    #[arg(id = "host", long, global = true)]
    pub host: Option<String>,

    /// SSH port
    #[arg(id = "port", long, global = true, default_value_t = 22)]
    pub port: u16,

    /// SSH username
    #[arg(id = "user", long, global = true, default_value = "root")]
    pub user: String,

    /// SSH password (or set `REMARKABLE_PASSWORD` env var)
    #[arg(id = "password", long, global = true, env = "REMARKABLE_PASSWORD")]
    pub password: Option<String>,

    /// SSH private key path
    #[arg(id = "key_file", long, global = true, default_value = "~/.ssh/id_rsa")]
    pub key_file: String,

    /// Output format
    #[arg(
        id = "format",
        long,
        global = true,
        value_enum,
        default_value_t = OutputFormat::Human
    )]
    pub format: OutputFormat,

    /// SSH connection timeout in seconds
    #[arg(id = "timeout", long, global = true, default_value_t = 5)]
    pub timeout: u64,

    /// Remote xochitl data directory path
    #[arg(
        id = "data_dir",
        long,
        global = true,
        default_value = "/home/root/.local/share/remarkable/xochitl"
    )]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CliValueSource {
    CommandLine,
    EnvVariable,
    #[default]
    DefaultValue,
    Unset,
}

impl CliValueSource {
    #[must_use]
    pub fn is_explicit(self) -> bool {
        matches!(self, Self::CommandLine | Self::EnvVariable)
    }
}

impl From<Option<ValueSource>> for CliValueSource {
    fn from(value: Option<ValueSource>) -> Self {
        match value {
            Some(ValueSource::CommandLine) => Self::CommandLine,
            Some(ValueSource::EnvVariable) => Self::EnvVariable,
            Some(ValueSource::DefaultValue | _) => Self::DefaultValue,
            None => Self::Unset,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalOptionSources {
    pub host: CliValueSource,
    pub port: CliValueSource,
    pub user: CliValueSource,
    pub password: CliValueSource,
    pub key_file: CliValueSource,
    pub format: CliValueSource,
    pub timeout: CliValueSource,
    pub data_dir: CliValueSource,
}

impl GlobalOptionSources {
    #[must_use]
    pub fn from_matches(matches: &ArgMatches) -> Self {
        Self {
            host: matches.value_source("host").into(),
            port: matches.value_source("port").into(),
            user: matches.value_source("user").into(),
            password: matches.value_source("password").into(),
            key_file: matches.value_source("key_file").into(),
            format: matches.value_source("format").into(),
            timeout: matches.value_source("timeout").into(),
            data_dir: matches.value_source("data_dir").into(),
        }
    }
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

    /// Filter by entry kind
    #[arg(long, value_enum, default_value_t = EntryKindFilter::All)]
    pub kind: EntryKindFilter,
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
    /// Local directory to save backup. The xochitl tree is copied under
    /// `<local_dir>/xochitl/`; the firmware version goes alongside as
    /// `<local_dir>/version`. Pre-existing local files outside the tree
    /// are left untouched (this is a backup, not a sync).
    pub local_dir: PathBuf,

    /// Only copy files whose remote mtime is newer than the local copy.
    /// First-run behaviour (target dir absent) is identical to a full
    /// backup.
    #[arg(long)]
    pub incremental: bool,

    /// Print the planned set of files without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct DownloadArgs {
    /// Path or UUID of the document to download
    pub path_or_uuid: String,

    /// Output path. For PDF/ePub a file path (default `./<name>.<ext>`);
    /// for notebooks a directory path (default `./<name>/`). Refuses to
    /// overwrite an existing destination.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Page range for notebooks (e.g., "1-5" or "1,3,7"). 1-indexed.
    /// Invalid for PDF/ePub.
    #[arg(long)]
    pub pages: Option<PageSelection>,
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
pub enum FindTypeFilter {
    Document,
    Folder,
    All,
}
