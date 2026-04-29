//! `RootTextBlock` (0x07) and the text-CRDT format — spec §8.

use crate::crdt::CrdtId;
use crate::error::ParseError;

/// Inline formatting code — spec §8.4. Lives on a text item with an empty
/// string body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InlineFormat {
    BoldStart = 1,
    BoldEnd = 2,
    ItalicStart = 3,
    ItalicEnd = 4,
}

/// Paragraph style — spec §8.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ParagraphStyle {
    Basic = 0,
    Plain = 1,
    Heading = 2,
    Bold = 3,
    Bullet = 4,
    Bullet2 = 5,
    CheckboxUnchecked = 6,
    CheckboxChecked = 7,
}

impl ParagraphStyle {
    pub fn from_u8(v: u8) -> Result<Self, ParseError> {
        Ok(match v {
            0 => Self::Basic,
            1 => Self::Plain,
            2 => Self::Heading,
            3 => Self::Bold,
            4 => Self::Bullet,
            5 => Self::Bullet2,
            6 => Self::CheckboxUnchecked,
            7 => Self::CheckboxChecked,
            other => return Err(ParseError::InvalidParagraphStyle(other)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TextItem {
    pub item_id: CrdtId,
    pub left_id: CrdtId,
    pub right_id: CrdtId,
    pub deleted_length: u32,
    pub content: Option<TextContent>,
}

#[derive(Debug, Clone)]
pub enum TextContent {
    String(String),
    Format(InlineFormat),
}

#[derive(Debug, Clone)]
pub struct ParagraphFormat {
    pub char_id: CrdtId,
    pub timestamp: CrdtId,
    pub style: ParagraphStyle,
}

#[derive(Debug, Clone)]
pub struct RootText {
    pub items: Vec<TextItem>,
    pub paragraphs: Vec<ParagraphFormat>,
    pub pos_x: f64,
    pub pos_y: f64,
    pub width: f32,
}
