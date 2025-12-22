//! DEC Special Graphics character set support for VT100/VT120 terminals.
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
pub enum DecGraphicsChar {
    /// 0x5f: Non-breaking space (NBSP)
    Nbsp,
    /// 0x60: Diamond (◆)
    Diamond,
    /// 0x61: Checkerboard/stipple (▒)
    Checkerboard,
    /// 0x62: Horizontal tab symbol (␉)
    HTab,
    /// 0x63: Form feed symbol (␌)
    FormFeed,
    /// 0x64: Carriage return symbol (␍)
    CarriageReturn,
    /// 0x65: Line feed symbol (␊)
    LineFeed,
    /// 0x66: Degree symbol (°)
    Degree,
    /// 0x67: Plus/minus (±)
    PlusMinus,
    /// 0x68: Newline symbol (␤)
    Newline,
    /// 0x69: Vertical tab symbol (␋)
    VTab,
    /// 0x6a: Lower right corner (┘)
    LowerRightCorner,
    /// 0x6b: Upper right corner (┐)
    UpperRightCorner,
    /// 0x6c: Upper left corner (┌)
    UpperLeftCorner,
    /// 0x6d: Lower left corner (└)
    LowerLeftCorner,
    /// 0x6e: Crossing lines (┼)
    Cross,
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
    /// 0x76: Bottom tee (┴)
    BottomTee,
    /// 0x77: Top tee (┬)
    TopTee,
    /// 0x78: Vertical line (│)
    VerticalLine,
    /// 0x79: Less than or equal (≤)
    LessOrEqual,
    /// 0x7a: Greater than or equal (≥)
    GreaterOrEqual,
    /// 0x7b: Pi (π)
    Pi,
    /// 0x7c: Not equal (≠)
    NotEqual,
    /// 0x7d: UK Pound sign (£)
    Pound,
    /// 0x7e: Centered dot / bullet (·)
    Bullet,
}

impl DecGraphicsChar {
    /// Returns the ASCII character code that produces this graphic when in DEC Special Graphics mode
    pub const fn as_dec_char(self) -> char {
        match self {
            Self::Nbsp => '\x5f',             // _
            Self::Diamond => '\x60',          // `
            Self::Checkerboard => '\x61',     // a
            Self::HTab => '\x62',             // b
            Self::FormFeed => '\x63',         // c
            Self::CarriageReturn => '\x64',   // d
            Self::LineFeed => '\x65',         // e
            Self::Degree => '\x66',           // f
            Self::PlusMinus => '\x67',        // g
            Self::Newline => '\x68',          // h
            Self::VTab => '\x69',             // i
            Self::LowerRightCorner => '\x6a', // j
            Self::UpperRightCorner => '\x6b', // k
            Self::UpperLeftCorner => '\x6c',  // l
            Self::LowerLeftCorner => '\x6d',  // m
            Self::Cross => '\x6e',            // n
            Self::ScanLine1 => '\x6f',        // o
            Self::ScanLine3 => '\x70',        // p
            Self::HorizontalLine => '\x71',   // q
            Self::ScanLine7 => '\x72',        // r
            Self::ScanLine9 => '\x73',        // s
            Self::LeftTee => '\x74',          // t
            Self::RightTee => '\x75',         // u
            Self::BottomTee => '\x76',        // v
            Self::TopTee => '\x77',           // w
            Self::VerticalLine => '\x78',     // x
            Self::LessOrEqual => '\x79',      // y
            Self::GreaterOrEqual => '\x7a',   // z
            Self::Pi => '\x7b',               // {
            Self::NotEqual => '\x7c',         // |
            Self::Pound => '\x7d',            // }
            Self::Bullet => '\x7e',           // ~
        }
    }

    /// Returns the Unicode equivalent of this DEC Special Graphics character
    pub const fn as_unicode(self) -> char {
        match self {
            Self::Nbsp => '\u{00A0}',             // Non-breaking space
            Self::Diamond => '◆',                // U+25C6
            Self::Checkerboard => '▒',           // U+2592
            Self::HTab => '␉',                   // U+2409
            Self::FormFeed => '␌',               // U+240C
            Self::CarriageReturn => '␍',         // U+240D
            Self::LineFeed => '␊',               // U+240A
            Self::Degree => '°',                 // U+00B0
            Self::PlusMinus => '±',              // U+00B1
            Self::Newline => '␤',                // U+2424
            Self::VTab => '␋',                   // U+240B
            Self::LowerRightCorner => '┘',       // U+2518
            Self::UpperRightCorner => '┐',       // U+2510
            Self::UpperLeftCorner => '┌',        // U+250C
            Self::LowerLeftCorner => '└',        // U+2514
            Self::Cross => '┼',                  // U+253C
            Self::ScanLine1 => '⎺',              // U+23BA
            Self::ScanLine3 => '⎻',              // U+23BB
            Self::HorizontalLine => '─',         // U+2500
            Self::ScanLine7 => '⎼',              // U+23BC
            Self::ScanLine9 => '⎽',              // U+23BD
            Self::LeftTee => '├',                // U+251C
            Self::RightTee => '┤',               // U+2524
            Self::BottomTee => '┴',              // U+2534
            Self::TopTee => '┬',                 // U+252C
            Self::VerticalLine => '│',           // U+2502
            Self::LessOrEqual => '≤',            // U+2264
            Self::GreaterOrEqual => '≥',         // U+2265
            Self::Pi => 'π',                     // U+03C0
            Self::NotEqual => '≠',               // U+2260
            Self::Pound => '£',                  // U+00A3
            Self::Bullet => '·',                 // U+00B7
        }
    }

    /// Convert from ASCII code (0x5f-0x7e) to DecGraphicsChar
    pub const fn from_ascii(code: u8) -> Option<Self> {
        match code {
            0x5f => Some(Self::Nbsp),
            0x60 => Some(Self::Diamond),
            0x61 => Some(Self::Checkerboard),
            0x62 => Some(Self::HTab),
            0x63 => Some(Self::FormFeed),
            0x64 => Some(Self::CarriageReturn),
            0x65 => Some(Self::LineFeed),
            0x66 => Some(Self::Degree),
            0x67 => Some(Self::PlusMinus),
            0x68 => Some(Self::Newline),
            0x69 => Some(Self::VTab),
            0x6a => Some(Self::LowerRightCorner),
            0x6b => Some(Self::UpperRightCorner),
            0x6c => Some(Self::UpperLeftCorner),
            0x6d => Some(Self::LowerLeftCorner),
            0x6e => Some(Self::Cross),
            0x6f => Some(Self::ScanLine1),
            0x70 => Some(Self::ScanLine3),
            0x71 => Some(Self::HorizontalLine),
            0x72 => Some(Self::ScanLine7),
            0x73 => Some(Self::ScanLine9),
            0x74 => Some(Self::LeftTee),
            0x75 => Some(Self::RightTee),
            0x76 => Some(Self::BottomTee),
            0x77 => Some(Self::TopTee),
            0x78 => Some(Self::VerticalLine),
            0x79 => Some(Self::LessOrEqual),
            0x7a => Some(Self::GreaterOrEqual),
            0x7b => Some(Self::Pi),
            0x7c => Some(Self::NotEqual),
            0x7d => Some(Self::Pound),
            0x7e => Some(Self::Bullet),
            _ => None,
        }
    }
}

/// Helper struct for building strings with DEC Special Graphics
pub struct DecGraphicsBuilder {
    buffer: String,
    in_graphics_mode: bool,
}

impl DecGraphicsBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            in_graphics_mode: false,
        }
    }

    /// Enter DEC Special Graphics mode
    pub fn enter_graphics(&mut self) -> &mut Self {
        if !self.in_graphics_mode {
            self.buffer.push_str(ENTER_DEC_GRAPHICS);
            self.in_graphics_mode = true;
        }
        self
    }

    /// Exit DEC Special Graphics mode (return to ASCII)
    pub fn exit_graphics(&mut self) -> &mut Self {
        if self.in_graphics_mode {
            self.buffer.push_str(EXIT_DEC_GRAPHICS);
            self.in_graphics_mode = false;
        }
        self
    }

    /// Add a DEC graphics character (automatically enters graphics mode)
    pub fn graphic(&mut self, ch: DecGraphicsChar) -> &mut Self {
        self.enter_graphics();
        self.buffer.push(ch.as_dec_char());
        self
    }

    /// Add multiple graphics characters
    pub fn graphics(&mut self, chars: &[DecGraphicsChar]) -> &mut Self {
        self.enter_graphics();
        for ch in chars {
            self.buffer.push(ch.as_dec_char());
        }
        self
    }

    /// Add a repeated graphics character
    pub fn repeat_graphic(&mut self, ch: DecGraphicsChar, count: usize) -> &mut Self {
        self.enter_graphics();
        for _ in 0..count {
            self.buffer.push(ch.as_dec_char());
        }
        self
    }

    /// Add ASCII text (automatically exits graphics mode)
    pub fn text(&mut self, s: &str) -> &mut Self {
        self.exit_graphics();
        self.buffer.push_str(s);
        self
    }

    /// Finish building and return the string (ensures we exit graphics mode)
    pub fn build(mut self) -> String {
        self.exit_graphics();
        self.buffer
    }
}

impl Default for DecGraphicsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Draw a horizontal line using DEC graphics
pub fn horizontal_line(width: usize) -> String {
    let mut builder = DecGraphicsBuilder::new();
    builder.repeat_graphic(DecGraphicsChar::HorizontalLine, width);
    builder.build()
}

/// Draw a vertical line segment using DEC graphics
pub fn vertical_line_char() -> String {
    let mut builder = DecGraphicsBuilder::new();
    builder.graphic(DecGraphicsChar::VerticalLine);
    builder.build()
}

/// Draw a simple box frame
pub fn draw_box(width: usize, height: usize) -> String {
    use DecGraphicsChar::*;

    let mut builder = DecGraphicsBuilder::new();

    // Top border
    builder.graphic(UpperLeftCorner);
    builder.repeat_graphic(HorizontalLine, width.saturating_sub(2));
    builder.graphic(UpperRightCorner);
    builder.text("\r\n");

    // Middle rows
    for _ in 0..height.saturating_sub(2) {
        builder.graphic(VerticalLine);
        builder.exit_graphics();
        for _ in 0..width.saturating_sub(2) {
            builder.buffer.push(' ');
        }
        builder.graphic(VerticalLine);
        builder.text("\r\n");
    }

    // Bottom border
    builder.graphic(LowerLeftCorner);
    builder.repeat_graphic(HorizontalLine, width.saturating_sub(2));
    builder.graphic(LowerRightCorner);

    builder.build()
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

    #[test]
    fn test_from_ascii() {
        assert_eq!(
            DecGraphicsChar::from_ascii(0x6c),
            Some(DecGraphicsChar::UpperLeftCorner)
        );
        assert_eq!(DecGraphicsChar::from_ascii(0x20), None);
    }

    #[test]
    fn test_builder() {
        let mut builder = DecGraphicsBuilder::new();
        builder
            .graphic(DecGraphicsChar::UpperLeftCorner)
            .repeat_graphic(DecGraphicsChar::HorizontalLine, 3)
            .graphic(DecGraphicsChar::UpperRightCorner);
        let result = builder.build();

        assert!(result.starts_with(ENTER_DEC_GRAPHICS));
        assert!(result.ends_with(EXIT_DEC_GRAPHICS));
        assert!(result.contains("lqqqk"));
    }
}
