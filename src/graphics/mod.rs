//! Graphics support for VT100/VT220/VT340 terminals.
//!
//! This module provides:
//! - DEC Special Graphics character set for box drawing
//! - DRCS (Dynamically Redefinable Character Set) for custom shading glyphs
//! - Sixel graphics for bitmap rendering (VT340)
//! - Cell-based frame representation for efficient differential rendering

mod cell;
mod dec;
mod drcs;
mod sixel;

pub use cell::{Cell, Frame, render_frame_diff};
pub use dec::{DecGraphicsChar, ENTER_DEC_GRAPHICS, EXIT_DEC_GRAPHICS};
pub use drcs::{SHIFT_IN, SHIFT_OUT, brightness_to_drcs_char, get_drcs_load_sequence};
pub use sixel::{SixelConfig, image_to_sixel};
