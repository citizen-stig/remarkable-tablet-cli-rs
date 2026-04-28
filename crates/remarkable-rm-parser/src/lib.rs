//! Parser and renderer for reMarkable `.rm` notebook page files.
//!
//! Notebook pages on the tablet are stored as a per-page binary format
//! (versions 3 through 6). This crate decodes the bytes into stroke data
//! and rasterizes the strokes to PNG.
//!
//! The format is documented in `RM_FORMAT_V6_SPEC.md` at the workspace
//! root. Implementation is deferred to Phase 4 of the project SPEC.

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("not yet implemented")]
    NotImplemented,
}
