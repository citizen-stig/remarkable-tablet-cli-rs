//! Primitive readers — all multi-byte values are little-endian per spec §2.
//!
//! Implementations land with Phase 4 step 10. The signatures here pin the
//! shape future code must follow.

use crate::error::ParseError;
use std::io::Read;

pub fn read_u8<R: Read>(_r: &mut R) -> Result<u8, ParseError> {
    todo!("spec §2 — primitive types")
}

pub fn read_u16<R: Read>(_r: &mut R) -> Result<u16, ParseError> {
    todo!()
}

pub fn read_u32<R: Read>(_r: &mut R) -> Result<u32, ParseError> {
    todo!()
}

pub fn read_i32<R: Read>(_r: &mut R) -> Result<i32, ParseError> {
    todo!()
}

pub fn read_f32<R: Read>(_r: &mut R) -> Result<f32, ParseError> {
    todo!()
}

pub fn read_f64<R: Read>(_r: &mut R) -> Result<f64, ParseError> {
    todo!()
}

pub fn read_bool<R: Read>(_r: &mut R) -> Result<bool, ParseError> {
    todo!()
}

/// LEB128 unsigned varint, up to 10 bytes (effectively `u64`). Spec §2.
pub fn read_varuint<R: Read>(_r: &mut R) -> Result<u64, ParseError> {
    todo!()
}
