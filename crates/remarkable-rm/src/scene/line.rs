//! Stroke (line) format — spec §7.

use crate::crdt::CrdtId;
use crate::error::ParseError;

/// Pen tools — spec §7.2. Values 9-11, 19-20, and 22 are reserved/invalid
/// and rejected by [`Pen::from_u32`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Pen {
    PaintbrushV1 = 0,
    PencilV1 = 1,
    BallpointV1 = 2,
    MarkerV1 = 3,
    FinelinerV1 = 4,
    HighlighterV1 = 5,
    Eraser = 6,
    MechanicalPencilV1 = 7,
    EraserAreaSelect = 8,
    PaintbrushV2 = 12,
    MechanicalPencilV2 = 13,
    PencilV2 = 14,
    BallpointV2 = 15,
    MarkerV2 = 16,
    FinelinerV2 = 17,
    HighlighterV2 = 18,
    Calligraphy = 21,
    Shader = 23,
}

impl Pen {
    pub fn from_u32(v: u32) -> Result<Self, ParseError> {
        Ok(match v {
            0 => Self::PaintbrushV1,
            1 => Self::PencilV1,
            2 => Self::BallpointV1,
            3 => Self::MarkerV1,
            4 => Self::FinelinerV1,
            5 => Self::HighlighterV1,
            6 => Self::Eraser,
            7 => Self::MechanicalPencilV1,
            8 => Self::EraserAreaSelect,
            12 => Self::PaintbrushV2,
            13 => Self::MechanicalPencilV2,
            14 => Self::PencilV2,
            15 => Self::BallpointV2,
            16 => Self::MarkerV2,
            17 => Self::FinelinerV2,
            18 => Self::HighlighterV2,
            21 => Self::Calligraphy,
            23 => Self::Shader,
            other => return Err(ParseError::InvalidPen(other)),
        })
    }
}

/// Pen colors — spec §7.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PenColor {
    Black = 0,
    Gray = 1,
    White = 2,
    Yellow = 3,
    Green = 4,
    Pink = 5,
    Blue = 6,
    Red = 7,
    GrayOverlap = 8,
    Highlight = 9,
    GreenV2 = 10,
    Cyan = 11,
    Magenta = 12,
    YellowV2 = 13,
}

impl PenColor {
    pub fn from_u32(v: u32) -> Result<Self, ParseError> {
        Ok(match v {
            0 => Self::Black,
            1 => Self::Gray,
            2 => Self::White,
            3 => Self::Yellow,
            4 => Self::Green,
            5 => Self::Pink,
            6 => Self::Blue,
            7 => Self::Red,
            8 => Self::GrayOverlap,
            9 => Self::Highlight,
            10 => Self::GreenV2,
            11 => Self::Cyan,
            12 => Self::Magenta,
            13 => Self::YellowV2,
            other => return Err(ParseError::InvalidPenColor(other)),
        })
    }
}

/// Single stroke point. On disk this is one of two physical layouts (v1 = 24
/// bytes, v2 = 14 bytes; spec §7.1). The parser normalizes both to this
/// in-memory shape — v1 floats are scaled into the v2 integer ranges so the
/// renderer can stay single-codepath.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub speed: u16,
    pub width: u16,
    pub direction: u8,
    pub pressure: u8,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub tool: Pen,
    pub color: PenColor,
    pub thickness_scale: f64,
    pub starting_length: f32,
    pub points: Vec<Point>,
    pub timestamp: CrdtId,
    pub move_id: Option<CrdtId>,
}
