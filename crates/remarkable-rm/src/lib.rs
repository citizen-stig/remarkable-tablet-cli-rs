//! Parser and renderer for reMarkable `.rm` notebook page files.
//!
//! Notebook pages on the tablet are stored as a per-page binary format
//! (versions 3 through 6). This crate decodes the bytes into stroke data
//! and rasterizes the strokes to PNG.
//!
//! The wire format is documented in `RM_FORMAT_V6_SPEC.md` at the workspace
//! root. The implementation matches that document section-by-section.
//!
//! Phase status: type skeleton in place; parsing bodies are Phase 4 step 10.

pub mod blocks;
pub mod crdt;
pub mod document;
pub mod error;
pub mod primitives;
pub mod scene;
pub mod tag;

pub use blocks::{BlockHeader, BlockType};
pub use crdt::{CrdtId, LwwValue};
pub use document::{Layer, Page};
pub use error::ParseError;
pub use scene::line::{Line, Pen, PenColor, Point};
