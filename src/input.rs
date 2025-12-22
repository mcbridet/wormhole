//! Input parsing for serial terminal.
//!
//! This module handles parsing of keyboard input from the serial terminal,
//! including escape sequences for special keys like arrows and Page Up/Down.

/// Parsed escape sequences from terminal input
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeSequence {
    /// Page Up key (ESC [ 5 ~)
    PageUp,
    /// Page Down key (ESC [ 6 ~)
    PageDown,
    /// Up arrow key (ESC [ A)
    ArrowUp,
    /// Down arrow key (ESC [ B)
    ArrowDown,
    /// Right arrow key (ESC [ C)
    ArrowRight,
    /// Left arrow key (ESC [ D)
    ArrowLeft,
    /// Unknown or incomplete sequence
    Unknown,
}

/// Input events from the terminal
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// A printable character (0x20-0x7E)
    Char(char),
    /// Enter/Return key
    Enter,
    /// Backspace or Delete
    Backspace,
    /// Tab key
    Tab,
    /// Ctrl+C
    CtrlC,
    /// Ctrl+R (Refresh)
    CtrlR,
    /// Escape sequence (arrow keys, page up/down, etc.)
    #[allow(dead_code)]
    Escape(EscapeSequence),
    /// Start of escape sequence (need more bytes)
    EscapeStart,
    /// Space bar (special handling in Call tab)
    Space,
    /// Byte that should be ignored
    Ignore,
}

/// Parser state for escape sequences
#[derive(Debug, Default)]
pub struct EscapeParser {
    buffer: Vec<u8>,
}

impl EscapeParser {
    /// Create a new escape parser
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Check if we're in the middle of parsing an escape sequence
    pub fn is_parsing(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Clear the escape buffer
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Feed a byte to the escape parser
    ///
    /// Returns `Some(EscapeSequence)` if a complete sequence was recognized,
    /// `None` if more bytes are needed.
    pub fn feed(&mut self, byte: u8) -> Option<EscapeSequence> {
        self.buffer.push(byte);

        // Check for complete sequences (minimum 3 bytes for arrow keys)
        if self.buffer.len() >= 3 {
            let seq = &self.buffer[..];

            // Page Up: ESC [ 5 ~
            if seq == b"\x1b[5~" {
                self.buffer.clear();
                return Some(EscapeSequence::PageUp);
            }

            // Page Down: ESC [ 6 ~
            if seq == b"\x1b[6~" {
                self.buffer.clear();
                return Some(EscapeSequence::PageDown);
            }

            // Arrow Up: ESC [ A
            if seq == b"\x1b[A" {
                self.buffer.clear();
                return Some(EscapeSequence::ArrowUp);
            }

            // Arrow Down: ESC [ B
            if seq == b"\x1b[B" {
                self.buffer.clear();
                return Some(EscapeSequence::ArrowDown);
            }

            // Arrow Right: ESC [ C
            if seq == b"\x1b[C" {
                self.buffer.clear();
                return Some(EscapeSequence::ArrowRight);
            }

            // Arrow Left: ESC [ D
            if seq == b"\x1b[D" {
                self.buffer.clear();
                return Some(EscapeSequence::ArrowLeft);
            }

            // Check for end of unknown sequence
            let last = seq[seq.len() - 1];
            if seq.len() > 6 || last == b'~' || (b'A'..=b'D').contains(&last) {
                self.buffer.clear();
                return Some(EscapeSequence::Unknown);
            }
        }

        // Need more bytes
        None
    }
}

/// Parse a single byte into an input event
///
/// Note: This does not handle escape sequences - use `EscapeParser` for those.
/// Returns `InputEvent::EscapeStart` when an escape byte is encountered.
pub fn parse_byte(byte: u8) -> InputEvent {
    match byte {
        0x1b => InputEvent::EscapeStart,
        b'\r' | b'\n' => InputEvent::Enter,
        0x7f | 0x08 => InputEvent::Backspace,
        0x09 => InputEvent::Tab,
        0x03 => InputEvent::CtrlC,
        0x12 => InputEvent::CtrlR,
        0x20 => InputEvent::Space,
        b if (0x21..0x7f).contains(&b) => InputEvent::Char(b as char),
        _ => InputEvent::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_printable() {
        assert_eq!(parse_byte(b'a'), InputEvent::Char('a'));
        assert_eq!(parse_byte(b'Z'), InputEvent::Char('Z'));
        assert_eq!(parse_byte(b'5'), InputEvent::Char('5'));
    }

    #[test]
    fn test_parse_control() {
        assert_eq!(parse_byte(b'\r'), InputEvent::Enter);
        assert_eq!(parse_byte(b'\n'), InputEvent::Enter);
        assert_eq!(parse_byte(0x7f), InputEvent::Backspace);
        assert_eq!(parse_byte(0x08), InputEvent::Backspace);
        assert_eq!(parse_byte(0x09), InputEvent::Tab);
        assert_eq!(parse_byte(0x03), InputEvent::CtrlC);
        assert_eq!(parse_byte(0x12), InputEvent::CtrlR);
    }

    #[test]
    fn test_escape_parser_arrows() {
        let mut parser = EscapeParser::new();

        // Arrow Up
        assert!(parser.feed(0x1b).is_none());
        assert!(parser.feed(b'[').is_none());
        assert_eq!(parser.feed(b'A'), Some(EscapeSequence::ArrowUp));
        assert!(!parser.is_parsing());

        // Arrow Down
        assert!(parser.feed(0x1b).is_none());
        assert!(parser.feed(b'[').is_none());
        assert_eq!(parser.feed(b'B'), Some(EscapeSequence::ArrowDown));

        // Arrow Right
        assert!(parser.feed(0x1b).is_none());
        assert!(parser.feed(b'[').is_none());
        assert_eq!(parser.feed(b'C'), Some(EscapeSequence::ArrowRight));

        // Arrow Left
        assert!(parser.feed(0x1b).is_none());
        assert!(parser.feed(b'[').is_none());
        assert_eq!(parser.feed(b'D'), Some(EscapeSequence::ArrowLeft));
    }

    #[test]
    fn test_escape_parser_page() {
        let mut parser = EscapeParser::new();

        // Page Up
        assert!(parser.feed(0x1b).is_none());
        assert!(parser.feed(b'[').is_none());
        assert!(parser.feed(b'5').is_none());
        assert_eq!(parser.feed(b'~'), Some(EscapeSequence::PageUp));

        // Page Down
        assert!(parser.feed(0x1b).is_none());
        assert!(parser.feed(b'[').is_none());
        assert!(parser.feed(b'6').is_none());
        assert_eq!(parser.feed(b'~'), Some(EscapeSequence::PageDown));
    }
}
