//! Chat buffer with scrollback support.

use std::collections::VecDeque;

use super::esc;
use super::{CHAT_REGION_START, CHAT_VISIBLE_LINES, MAX_SCROLLBACK};
use crate::graphics::{DecGraphicsChar, ENTER_DEC_GRAPHICS, EXIT_DEC_GRAPHICS};

/// Calculate visible length of a string (ignoring escape codes)
pub(crate) fn visible_len(s: &str) -> usize {
    s.chars().filter(|&c| c != '\x0E' && c != '\x0F').count()
}

/// Chat buffer with scrollback support
pub struct ChatBuffer {
    /// All messages in the buffer
    lines: VecDeque<String>,
    /// Current scroll offset (0 = viewing most recent, >0 = scrolled up)
    scroll_offset: usize,
    /// Terminal width for wrapping
    width: usize,
}

impl ChatBuffer {
    /// Create a new chat buffer
    pub fn new(width: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(MAX_SCROLLBACK),
            scroll_offset: 0,
            width,
        }
    }

    /// Check if the buffer has enough lines to fill the screen
    pub fn is_full(&self) -> bool {
        self.lines.len() > CHAT_VISIBLE_LINES
    }

    /// Append a character, handling wrapping with indentation
    /// Returns true if a new line was created or modified (requiring multi-line redraw)
    pub fn type_char(&mut self, ch: char, indent: &str) -> bool {
        let max_len = self.width - 4;

        if self.lines.is_empty() {
            self.lines.push_back(String::new());
        }

        let last_idx = self.lines.len() - 1;
        let current_len = visible_len(&self.lines[last_idx]);

        if current_len + 1 > max_len {
            // Need to wrap
            let mut word_to_move = String::new();
            let mut truncated_line = String::new();
            let mut moved = false;

            // Try word wrapping if not whitespace
            if !ch.is_whitespace() {
                let last_line = &self.lines[last_idx];
                if let Some(last_space) = last_line.rfind(' ') {
                    // Only move if it's not the whole line and not too long
                    if last_line.len() - last_space < max_len / 2 {
                        word_to_move = last_line[last_space + 1..].to_string();
                        truncated_line = last_line[..last_space].to_string();
                        moved = true;
                    }
                }
            }

            if moved {
                self.lines[last_idx] = truncated_line;
                let mut new_line = String::from(indent);
                new_line.push_str(&word_to_move);
                new_line.push(ch);
                self.push_raw(new_line);
            } else {
                // Character wrap
                let mut new_line = String::from(indent);
                new_line.push(ch);
                self.push_raw(new_line);
            }
            true
        } else {
            self.lines[last_idx].push(ch);
            false
        }
    }

    /// Add a message to the buffer, wrapping if necessary
    pub fn push(&mut self, message: String) {
        if message.is_empty() {
            self.push_raw(String::new());
            return;
        }

        let max_len = self.width - 4; // "│ " on left, " │" on right

        for line in message.lines() {
            let mut current_line = String::new();
            let mut first_word = true;

            for word in line.split(' ') {
                let space_len = if first_word { 0 } else { 1 };
                let word_len = word.len();

                if current_line.len() + space_len + word_len > max_len {
                    // Line full, push it
                    if !current_line.is_empty() {
                        self.push_raw(current_line);
                        current_line = String::new();
                        // first_word becomes true for the new line, but we immediately add the current word
                        // so it will become false again at the end of this iteration.
                    }

                    // Now handle the word
                    if word.len() > max_len {
                        // Word too long, split it
                        let mut remaining = word;
                        while remaining.len() > max_len {
                            self.push_raw(remaining[..max_len].to_string());
                            remaining = &remaining[max_len..];
                        }
                        current_line.push_str(remaining);
                        first_word = false;
                    } else {
                        // Word fits on new line
                        current_line.push_str(word);
                        first_word = false;
                    }
                } else {
                    // Fits on current line
                    if !first_word {
                        current_line.push(' ');
                    }
                    current_line.push_str(word);
                    first_word = false;
                }
            }

            // Push the last line
            if !current_line.is_empty() || line.is_empty() {
                self.push_raw(current_line);
            }
        }
    }

    /// Internal helper to push a single line and handle capacity
    fn push_raw(&mut self, line: String) {
        self.lines.push_back(line);

        // Remove old lines if over capacity
        while self.lines.len() > MAX_SCROLLBACK {
            self.lines.pop_front();
            // Adjust scroll offset if we removed lines we were viewing
            if self.scroll_offset > 0 {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
        }
    }

    /// Update the last line in the buffer (useful for streaming)
    pub fn update_last_line(&mut self, content: &str) {
        let max_len = self.width - 4;
        let truncated = if content.len() > max_len {
            content[..max_len].to_string()
        } else {
            content.to_string()
        };

        if let Some(last) = self.lines.back_mut() {
            *last = truncated;
        }
    }

    /// Clear the chat buffer
    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll_offset = 0;
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        let max_offset = self.lines.len().saturating_sub(CHAT_VISIBLE_LINES);
        self.scroll_offset = (self.scroll_offset + n).min(max_offset);
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll to bottom (most recent messages)
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Get the lines currently visible in the display window
    fn visible_lines(&self) -> Vec<&str> {
        let total = self.lines.len();
        if total == 0 {
            return vec![];
        }

        // Calculate the range of lines to show
        // scroll_offset=0 means show the last CHAT_VISIBLE_LINES
        // scroll_offset=N means show N lines earlier
        let end = total.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(CHAT_VISIBLE_LINES);

        self.lines
            .iter()
            .skip(start)
            .take(end - start)
            .map(|s| s.as_str())
            .collect()
    }

    /// Render the last n visible lines
    pub fn render_bottom_lines(&self, n: usize) -> String {
        use DecGraphicsChar::VerticalLine;

        let visible = self.visible_lines();
        if visible.is_empty() {
            return String::new();
        }

        let count = n.min(visible.len());
        let start_idx = visible.len() - count;

        let mut output = String::new();
        output.push_str(esc::SAVE_CURSOR);

        for i in 0..count {
            let row_idx = start_idx + i;
            let screen_row = CHAT_REGION_START + row_idx;
            let line = visible[row_idx];
            let max_len = self.width - 4;

            output.push_str(&esc::cursor_to(screen_row, 1));

            // Left border
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push(' ');

            // Content
            output.push_str(line);
            // Pad
            let vis_len = visible_len(line);
            for _ in vis_len..max_len {
                output.push(' ');
            }

            // Right border
            output.push(' ');
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }

        output.push_str(esc::RESTORE_CURSOR);
        output
    }

    /// Render only the last visible line (optimization for streaming)
    pub fn render_last_line(&self) -> String {
        use DecGraphicsChar::VerticalLine;

        let visible = self.visible_lines();
        if visible.is_empty() {
            return String::new();
        }

        let row_idx = visible.len() - 1;
        let screen_row = CHAT_REGION_START + row_idx;
        let line = visible[row_idx];
        let max_len = self.width - 4;

        let mut output = String::new();
        output.push_str(esc::SAVE_CURSOR);
        output.push_str(&esc::cursor_to(screen_row, 1));

        // Left border
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(VerticalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);
        output.push(' ');

        // Content
        output.push_str(line);
        // Pad
        let vis_len = visible_len(line);
        for _ in vis_len..max_len {
            output.push(' ');
        }

        // Right border
        output.push(' ');
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(VerticalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);

        output.push_str(esc::RESTORE_CURSOR);
        output
    }

    /// Render the entire chat area
    pub fn render(&self) -> String {
        use DecGraphicsChar::VerticalLine;

        let mut output = String::new();
        let visible = self.visible_lines();
        let max_len = self.width - 4;

        // Save cursor
        output.push_str(esc::SAVE_CURSOR);

        // Draw each row in the chat area
        for row_idx in 0..CHAT_VISIBLE_LINES {
            let screen_row = CHAT_REGION_START + row_idx;
            output.push_str(&esc::cursor_to(screen_row, 1));

            // Left border
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push(' ');

            // Content or empty
            if row_idx < visible.len() {
                let line = visible[row_idx];
                output.push_str(line);
                // Pad to clear old content
                let vis_len = visible_len(line);
                for _ in vis_len..max_len {
                    output.push(' ');
                }
            } else {
                // Empty line
                for _ in 0..max_len {
                    output.push(' ');
                }
            }

            // Right border
            output.push(' ');
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }

        // Restore cursor
        output.push_str(esc::RESTORE_CURSOR);

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_len() {
        assert_eq!(visible_len("hello"), 5);
        assert_eq!(visible_len("hello world"), 11);
        // Shift in/out characters should not count
        assert_eq!(visible_len("a\x0Eb\x0Fc"), 3);
    }

    #[test]
    fn test_new_buffer() {
        let buf = ChatBuffer::new(80);
        assert!(!buf.is_full());
        assert_eq!(buf.visible_lines().len(), 0);
    }

    #[test]
    fn test_push_simple() {
        let mut buf = ChatBuffer::new(80);
        buf.push("Hello, world!".to_string());
        assert_eq!(buf.visible_lines(), vec!["Hello, world!"]);
    }

    #[test]
    fn test_push_wrapping() {
        let mut buf = ChatBuffer::new(20); // Very narrow, max_len = 16
        buf.push("This is a long message that should wrap".to_string());
        let lines = buf.visible_lines();
        assert!(lines.len() > 1, "Message should wrap to multiple lines");
        // Each line should be at most max_len (width - 4 = 16)
        for line in &lines {
            assert!(line.len() <= 16, "Line too long: {}", line);
        }
    }

    #[test]
    fn test_scroll() {
        let mut buf = ChatBuffer::new(80);
        // Push more lines than visible area
        for i in 0..30 {
            buf.push(format!("Line {}", i));
        }

        // Should start at bottom
        let visible = buf.visible_lines();
        assert!(visible.last().unwrap().contains("29"));

        // Scroll up
        buf.scroll_up(5);
        let visible = buf.visible_lines();
        assert!(!visible.last().unwrap().contains("29"));

        // Scroll back to bottom
        buf.scroll_to_bottom();
        let visible = buf.visible_lines();
        assert!(visible.last().unwrap().contains("29"));
    }

    #[test]
    fn test_clear() {
        let mut buf = ChatBuffer::new(80);
        buf.push("Test message".to_string());
        assert!(!buf.visible_lines().is_empty());
        buf.clear();
        assert!(buf.visible_lines().is_empty());
    }
}
