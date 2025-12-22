//! Graphics support for VT100/VT220 terminals.
//!
//! This module provides:
//! - DEC Special Graphics character set for box drawing
//! - DRCS (Dynamically Redefinable Character Set) for custom shading glyphs

mod dec;
mod drcs;

pub use dec::{DecGraphicsChar, ENTER_DEC_GRAPHICS, EXIT_DEC_GRAPHICS};
pub use drcs::{SHIFT_IN, SHIFT_OUT, brightness_to_drcs_char, get_drcs_load_sequence};
