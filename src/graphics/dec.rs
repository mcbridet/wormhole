//! DEC Special Graphics character set support for VT100/VT220 terminals.
//!
//! The DEC Special Graphics character set provides box-drawing characters,
//! line graphics, and other special symbols used on VT100-series terminals.
//!
//! To enter DEC Special Graphics mode, send ESC ( 0
//! To return to ASCII mode, send ESC ( B

/// Escape sequence to switch to DEC Special Graphics character set (G0)
pub const ENTER_DEC_GRAPHICS: &str = "\x1b(0";

/// Escape sequence to switch back to ASCII character set (G0)
pub const EXIT_DEC_GRAPHICS: &str = "\x1b(B";

/// DEC Special Graphics characters mapped from ASCII codes 0x5f-0x7e
/// when in graphics mode. The index is (ascii_code - 0x5f).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DecGraphicsChar {
    /// 0x60: Diamond (◆)
    Diamond,
    /// 0x61: Checkerboard/stipple (▒)
    Checkerboard,
    /// 0x66: Degree symbol (°)
    Degree,
    /// 0x67: Plus/minus (±)
    PlusMinus,
    /// 0x6a: Lower right corner (┘)
    LowerRightCorner,
    /// 0x6b: Upper right corner (┐)
    UpperRightCorner,
    /// 0x6c: Upper left corner (┌)
    UpperLeftCorner,
    /// 0x6d: Lower left corner (└)
    LowerLeftCorner,
    /// 0x6f: Horizontal line - scan 1 (⎺)
    ScanLine1,
    /// 0x70: Horizontal line - scan 3 (⎻)
    ScanLine3,
    /// 0x71: Horizontal line - scan 5 / box drawing horizontal (─)
    HorizontalLine,
    /// 0x72: Horizontal line - scan 7 (⎼)
    ScanLine7,
    /// 0x73: Horizontal line - scan 9 (⎽)
    ScanLine9,
    /// 0x74: Left tee (├)
    LeftTee,
    /// 0x75: Right tee (┤)
    RightTee,
    /// 0x78: Vertical line (│)
    VerticalLine,
    /// 0x7e: Centered dot / bullet (·)
    Bullet,
}

impl DecGraphicsChar {
    /// Returns the ASCII character code that produces this graphic when in DEC Special Graphics mode
    pub const fn as_dec_char(self) -> char {
        match self {
            Self::Diamond => '\x60',          // `
            Self::Checkerboard => '\x61',     // a
            Self::Degree => '\x66',           // f
            Self::PlusMinus => '\x67',        // g
            Self::LowerRightCorner => '\x6a', // j
            Self::UpperRightCorner => '\x6b', // k
            Self::UpperLeftCorner => '\x6c',  // l
            Self::LowerLeftCorner => '\x6d',  // m
            Self::ScanLine1 => '\x6f',        // o
            Self::ScanLine3 => '\x70',        // p
            Self::HorizontalLine => '\x71',   // q
            Self::ScanLine7 => '\x72',        // r
            Self::ScanLine9 => '\x73',        // s
            Self::LeftTee => '\x74',          // t
            Self::RightTee => '\x75',         // u
            Self::VerticalLine => '\x78',     // x
            Self::Bullet => '\x7e',           // ~
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_sequences() {
        assert_eq!(ENTER_DEC_GRAPHICS, "\x1b(0");
        assert_eq!(EXIT_DEC_GRAPHICS, "\x1b(B");
    }

    #[test]
    fn test_char_mapping() {
        assert_eq!(DecGraphicsChar::UpperLeftCorner.as_dec_char(), 'l');
        assert_eq!(DecGraphicsChar::UpperRightCorner.as_dec_char(), 'k');
        assert_eq!(DecGraphicsChar::LowerLeftCorner.as_dec_char(), 'm');
        assert_eq!(DecGraphicsChar::LowerRightCorner.as_dec_char(), 'j');
        assert_eq!(DecGraphicsChar::HorizontalLine.as_dec_char(), 'q');
        assert_eq!(DecGraphicsChar::VerticalLine.as_dec_char(), 'x');
    }
}
