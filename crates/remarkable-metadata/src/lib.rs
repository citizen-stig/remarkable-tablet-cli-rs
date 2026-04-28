//! Pure-data layer for reMarkable tablet metadata.
//!
//! Parses `.metadata` and `.content` JSON files into typed structures, builds
//! a logical document tree from a flat list of entries, and resolves
//! human-readable paths or UUIDs to entries. No I/O — feed it bytes (or
//! pre-parsed entries) and it returns data.
//!
//! The companion `remarkable-tablet` crate fetches these files over SSH/SFTP
//! and hands the raw bytes here for parsing.

pub mod error;
pub mod metadata;
pub mod page_range;
pub mod path_resolver;
pub mod sort;
pub mod tree;

pub use error::MetadataError;
pub use metadata::{
    DocumentEntry, FileType, ItemKind, ItemType, Parent, RawContent, RawMetadata, extract_uuid,
    parse_content, parse_metadata,
};
pub use page_range::{PageSelection, PageSelectionError};
pub use path_resolver::{Resolved, resolve, resolve_path, resolve_uuid_to_path};
pub use sort::SortField;
pub use tree::{ChildLookup, DocumentTree, EntryKindFilter, ListFilter, sort_entries};
