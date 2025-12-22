//! VT100/VT220 terminal control sequences and UI rendering.

use crate::dec_graphics::{DecGraphicsChar, ENTER_DEC_GRAPHICS, EXIT_DEC_GRAPHICS};
use std::collections::VecDeque;

/// Escape sequence to switch to 132 column mode
pub const ENTER_132_COL_MODE: &str = "\x1b[?3h";
/// Escape sequence to switch to 80 column mode
pub const EXIT_132_COL_MODE: &str = "\x1b[?3l";

/// Get the initialization sequence for the terminal
pub fn get_init_sequence(use_drcs: bool, use_132_cols: bool) -> String {
    let mut output = String::new();
    if use_132_cols {
        output.push_str(ENTER_132_COL_MODE);
    } else {
        output.push_str(EXIT_132_COL_MODE);
    }

    if use_drcs {
        output.push_str(&crate::drcs::get_drcs_load_sequence());
    }
    output
}

/// VT220 terminal dimensions (80x24 is standard)
pub const TERMINAL_HEIGHT: usize = 24;

/// Layout with borders:
/// Row 1: Top border with tabs
/// Rows 2-20: Chat display area (19 lines)
/// Row 21: Separator border
/// Rows 22-23: Input area (2 lines for wrapped input)
/// Row 24: Bottom border
pub const CHAT_REGION_START: usize = 2;
pub const CHAT_REGION_END: usize = 20;
pub const CHAT_VISIBLE_LINES: usize = CHAT_REGION_END - CHAT_REGION_START + 1; // 19 lines
pub const CALL_REGION_END: usize = 23;
pub const CALL_VISIBLE_LINES: usize = CALL_REGION_END - CHAT_REGION_START + 1; // 22 lines
pub const INPUT_ROW_START: usize = 22;
pub const INPUT_ROW_END: usize = 23;
pub const INPUT_ROWS: usize = INPUT_ROW_END - INPUT_ROW_START + 1; // 2 rows

/// Input area dimensions
/// Each row has: left border (1) + content (width-4) + space (1) + right border (1) = width visible
/// First row also has prompt taking some space
/// Maximum scrollback buffer size
pub const MAX_SCROLLBACK: usize = 10_000;

/// Tab identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Chat = 0,
    Call = 1,
    Gemini = 2,
}

impl Tab {
    pub fn next(self, gemini_available: bool, call_active: bool) -> Self {
        match self {
            Tab::Chat => {
                if call_active {
                    Tab::Call
                } else if gemini_available {
                    Tab::Gemini
                } else {
                    Tab::Chat
                }
            }
            Tab::Call => {
                if gemini_available {
                    Tab::Gemini
                } else {
                    Tab::Chat
                }
            }
            Tab::Gemini => Tab::Chat,
        }
    }
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

/// Calculate visible length of a string (ignoring escape codes)
fn visible_len(s: &str) -> usize {
    s.chars().filter(|&c| c != '\x0E' && c != '\x0F').count()
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

/// Render a stream frame to the content area
pub fn render_stream(
    _sender: &str,
    lines: &[String],
    prev_lines: Option<&Vec<String>>,
    width: usize,
) -> String {
    let mut output = String::new();

    // Calculate centering
    let frame_height = lines.len();
    let frame_width = if frame_height > 0 {
        let line = &lines[0];
        visible_len(line)
    } else {
        0
    };

    // Use integer division for centering, but ensure we don't start before CHAT_REGION_START
    let start_row = CHAT_REGION_START + (CALL_VISIBLE_LINES.saturating_sub(frame_height)) / 2;
    let start_col = (width.saturating_sub(frame_width)) / 2 + 1; // 1-based

    // Check if we can do differential rendering
    let can_diff = if let Some(prev) = prev_lines {
        // Only diff if dimensions match (centering hasn't changed)
        if prev.len() == lines.len() && !lines.is_empty() && !prev.is_empty() {
            visible_len(&prev[0]) == visible_len(&lines[0])
        } else {
            false
        }
    } else {
        false
    };

    // Draw frame
    for (i, line) in lines.iter().enumerate() {
        let row = start_row + i;
        if row >= CALL_REGION_END {
            break;
        }

        if can_diff {
            let prev_line = &prev_lines.unwrap()[i];
            if line == prev_line {
                continue; // Skip identical lines
            }
            // If line changed, redraw the whole line.
            // We previously tried to diff segments within the line, but this is
            // unreliable with hybrid ASCII/DEC graphics where escape codes are interspersed.
        }

        // Full redraw of the line
        output.push_str(&esc::cursor_to(row, start_col));
        output.push_str(line);
    }

    output
}

/// ANSI/VT100 escape sequences
pub mod esc {
    /// Clear entire screen
    pub const CLEAR_SCREEN: &str = "\x1b[2J";

    /// Move cursor to home position (1,1)
    pub const CURSOR_HOME: &str = "\x1b[H";

    /// Hide cursor
    pub const CURSOR_HIDE: &str = "\x1b[?25l";

    /// Show cursor
    pub const CURSOR_SHOW: &str = "\x1b[?25h";

    /// Reset all attributes
    pub const RESET_ATTRS: &str = "\x1b[0m";

    /// Reverse video
    pub const REVERSE: &str = "\x1b[7m";

    /// Save cursor position
    pub const SAVE_CURSOR: &str = "\x1b7";

    /// Restore cursor position
    pub const RESTORE_CURSOR: &str = "\x1b8";

    /// Move cursor to specific position (1-indexed)
    pub fn cursor_to(row: usize, col: usize) -> String {
        format!("\x1b[{};{}H", row, col)
    }

    /// Reset scroll region to full screen
    pub fn reset_scroll_region() -> String {
        "\x1b[r".to_string()
    }
}

/// Draw a horizontal line with optional left/right connectors
fn draw_horizontal_line(left: DecGraphicsChar, right: DecGraphicsChar, width: usize) -> String {
    use DecGraphicsChar::HorizontalLine;

    let mut output = String::new();
    output.push_str(ENTER_DEC_GRAPHICS);
    output.push(left.as_dec_char());
    for _ in 0..width - 2 {
        output.push(HorizontalLine.as_dec_char());
    }
    output.push(right.as_dec_char());
    output.push_str(EXIT_DEC_GRAPHICS);
    output
}

/// Draw the top border with tab indicators
fn draw_tab_bar(
    active_tab: Tab,
    gemini_available: bool,
    active_call: Option<&str>,
    width: usize,
) -> String {
    use DecGraphicsChar::*;

    let mut output = String::new();
    output.push_str(&esc::cursor_to(1, 1));

    // Start with upper left corner
    output.push_str(ENTER_DEC_GRAPHICS);
    output.push(UpperLeftCorner.as_dec_char());
    output.push_str(EXIT_DEC_GRAPHICS);

    // Helper to draw a tab
    let draw_tab = |label: &str, is_active: bool, is_next: bool| -> String {
        if is_active {
            format!("{}[{}]{}", esc::REVERSE, label, esc::RESET_ATTRS)
        } else if is_next {
            format!(" {} <Tab> ", label)
        } else {
            format!(" {} ", label)
        }
    };

    // Determine next tab for hint
    let next_tab = active_tab.next(gemini_available, active_call.is_some());

    // Chat Tab
    output.push_str(&draw_tab(
        "Chat",
        active_tab == Tab::Chat,
        next_tab == Tab::Chat,
    ));

    // Call Tab (if active)
    if let Some(peer_name) = active_call {
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(HorizontalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);

        let label = format!("Call ({})", peer_name);
        output.push_str(&draw_tab(
            &label,
            active_tab == Tab::Call,
            next_tab == Tab::Call,
        ));
    }

    // AI Tab (if available)
    if gemini_available {
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(HorizontalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);

        output.push_str(&draw_tab(
            "AI",
            active_tab == Tab::Gemini,
            next_tab == Tab::Gemini,
        ));
    }

    // Hints: ^Refresh / ^Clear
    let hints = " ^Refresh / ^Clear ";

    // Calculate used length
    let mut visible_len = 1; // Corner

    // Chat
    visible_len += if active_tab == Tab::Chat {
        6
    } else if next_tab == Tab::Chat {
        12
    } else {
        6
    };

    // Call
    if let Some(peer_name) = active_call {
        visible_len += 1; // Separator
        let label_len = 7 + peer_name.len();
        let tab_len = label_len + 2;
        let next_len = label_len + 8;
        visible_len += if active_tab == Tab::Call {
            tab_len
        } else if next_tab == Tab::Call {
            next_len
        } else {
            tab_len
        };
    }

    // AI
    if gemini_available {
        visible_len += 1; // Separator
        visible_len += if active_tab == Tab::Gemini {
            4
        } else if next_tab == Tab::Gemini {
            10
        } else {
            4
        };
    }

    visible_len += hints.len();
    visible_len += 1; // Right corner

    // Fill with horizontal line
    let remaining = width.saturating_sub(visible_len);

    output.push_str(ENTER_DEC_GRAPHICS);
    for _ in 0..remaining {
        output.push(HorizontalLine.as_dec_char());
    }
    output.push_str(EXIT_DEC_GRAPHICS);

    output.push_str(hints);

    output.push_str(ENTER_DEC_GRAPHICS);
    output.push(UpperRightCorner.as_dec_char());
    output.push_str(EXIT_DEC_GRAPHICS);

    output
}

/// Redraw just the tab bar (for switching tabs without full redraw)
pub fn redraw_tab_bar(
    active_tab: Tab,
    gemini_available: bool,
    active_call: Option<&str>,
    width: usize,
) -> String {
    let mut output = String::new();
    output.push_str(esc::SAVE_CURSOR);
    output.push_str(&draw_tab_bar(
        active_tab,
        gemini_available,
        active_call,
        width,
    ));
    output.push_str(esc::RESTORE_CURSOR);
    output
}

/// Calculate the maximum input length based on prompt size
pub fn max_input_length(client_name: &str, width: usize) -> usize {
    let prompt = format!("[{}] ", client_name);
    let prompt_len = prompt.len();
    let input_content_width = width - 4;

    // First row: content width minus prompt
    let first_row_capacity = input_content_width - prompt_len;
    // Subsequent rows: full content width
    let other_rows_capacity = input_content_width * (INPUT_ROWS - 1);

    first_row_capacity + other_rows_capacity
}

/// Initialize the split-screen UI with borders and tab support
pub fn init_split_screen_with_tabs(
    client_name: &str,
    active_tab: Tab,
    gemini_available: bool,
    active_call: Option<&str>,
    call_status: Option<&str>,
    width: usize,
) -> String {
    use DecGraphicsChar::*;

    let prompt = format!("[{}] ", client_name);
    let mut output = String::new();

    // Clear screen
    output.push_str(esc::CLEAR_SCREEN);
    output.push_str(esc::CURSOR_HOME);

    // Row 1: Top border with tabs
    output.push_str(&draw_tab_bar(
        active_tab,
        gemini_available,
        active_call,
        width,
    ));

    if active_tab == Tab::Call {
        // Draw full box for Call (no split)
        // Rows 2-23: Left and right borders
        for row in 2..=23 {
            output.push_str(&esc::cursor_to(row, 1));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push_str(&esc::cursor_to(row, width));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }

        // Row 24: Bottom border
        output.push_str(&esc::cursor_to(24, 1));
        output.push_str(&draw_horizontal_line(
            LowerLeftCorner,
            LowerRightCorner,
            width,
        ));

        // Draw status message if provided
        if let Some(status) = call_status {
            output.push_str(&esc::cursor_to(23, 3)); // Inside the box
            output.push_str(status);
        }

        // Hide cursor
        output.push_str(esc::CURSOR_HIDE);
    } else {
        // Rows 2-19: Left and right borders for chat area
        for row in CHAT_REGION_START..=CHAT_REGION_END {
            output.push_str(&esc::cursor_to(row, 1));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push_str(&esc::cursor_to(row, width));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }

        // Row 21: Separator ├────────────────────┤
        output.push_str(&esc::cursor_to(21, 1));
        output.push_str(&draw_horizontal_line(LeftTee, RightTee, width));

        // Rows 21-23: Input area borders
        for row in INPUT_ROW_START..=INPUT_ROW_END {
            output.push_str(&esc::cursor_to(row, 1));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push_str(&esc::cursor_to(row, width));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }

        // Draw prompt on first input row
        output.push_str(&esc::cursor_to(INPUT_ROW_START, 2));
        output.push_str(&prompt);

        // Row 24: Bottom border └────────────────────┘
        output.push_str(&esc::cursor_to(24, 1));
        output.push_str(&draw_horizontal_line(
            LowerLeftCorner,
            LowerRightCorner,
            width,
        ));

        // No scroll region - we manage scrolling ourselves via ChatBuffer

        // Position cursor at input area (after prompt)
        output.push_str(&esc::cursor_to(INPUT_ROW_START, 2 + prompt.len()));

        // Show cursor
        output.push_str(esc::CURSOR_SHOW);
    }

    output
}

/// Redraw the input line with current buffer content and cursor position
pub fn redraw_input(client_name: &str, buffer: &str, cursor_pos: usize, width: usize) -> String {
    use DecGraphicsChar::VerticalLine;

    let prompt = format!("[{}] ", client_name);
    let prompt_len = prompt.chars().count();
    let mut output = String::new();
    let input_content_width = width - 4;

    // Calculate capacity for each row
    let first_row_capacity = input_content_width - prompt_len;

    // Split buffer into rows
    let mut remaining = buffer;
    let mut row_contents: Vec<&str> = Vec::new();

    // Helper to find byte index for split
    let get_split_idx = |s: &str, cap: usize| -> usize {
        s.char_indices().map(|(i, _)| i).nth(cap).unwrap_or(s.len())
    };

    // First row gets less space due to prompt
    let split_idx = get_split_idx(remaining, first_row_capacity);
    row_contents.push(&remaining[..split_idx]);
    remaining = &remaining[split_idx..];

    // Subsequent rows get full width
    for _ in 1..INPUT_ROWS {
        if remaining.is_empty() {
            row_contents.push("");
        } else {
            let split_idx = get_split_idx(remaining, input_content_width);
            row_contents.push(&remaining[..split_idx]);
            remaining = &remaining[split_idx..];
        }
    }

    // Draw each input row
    for (i, content) in row_contents.iter().enumerate() {
        let row = INPUT_ROW_START + i;

        // Move to row, draw left border
        output.push_str(&esc::cursor_to(row, 1));
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(VerticalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);

        // First row has prompt
        if i == 0 {
            output.push_str(&prompt);
            output.push_str(content);
            // Pad to clear old content
            let content_len = content.chars().count();
            for _ in content_len..first_row_capacity {
                output.push(' ');
            }
        } else {
            output.push_str(content);
            // Pad to clear old content
            let content_len = content.chars().count();
            for _ in content_len..input_content_width {
                output.push(' ');
            }
        }

        // Draw right border
        output.push(' ');
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(VerticalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);
    }

    // Calculate cursor position
    // cursor_pos is index in buffer (0 to buffer.len())
    let (cursor_row, cursor_col) = if cursor_pos <= first_row_capacity {
        // Cursor on first row
        (INPUT_ROW_START, 2 + prompt_len + cursor_pos)
    } else {
        // Calculate which row and column
        let chars_after_first = cursor_pos - first_row_capacity;
        let mut row_index = 1 + chars_after_first / input_content_width;
        let mut col_in_row = chars_after_first % input_content_width;

        // Clamp to last row if we go past it (e.g. cursor at very end of full buffer)
        if row_index >= INPUT_ROWS {
            row_index = INPUT_ROWS - 1;
            col_in_row = input_content_width;
        }

        (INPUT_ROW_START + row_index, 2 + col_in_row)
    };

    output.push_str(&esc::cursor_to(cursor_row, cursor_col));

    output
}

/// Cleanup: reset scroll region before exit
pub fn cleanup_split_screen(width: usize) -> String {
    let mut output = String::new();
    output.push_str(&esc::reset_scroll_region());
    output.push_str(esc::CLEAR_SCREEN);

    let sad_mac = [
        " .-------. ",
        " | X   X | ",
        " |   L   | ",
        " |  ___  | ",
        " | /   \\ | ",
        " '-------' ",
    ];

    let messages = [
        "Wormhole server is not running.",
        "Please check logs or restart the device.",
    ];

    let total_lines = sad_mac.len() + 2 + messages.len(); // +2 for spacing
    let start_row = (TERMINAL_HEIGHT - total_lines) / 2;

    for (i, line) in sad_mac.iter().enumerate() {
        let padding = (width - line.len()) / 2;
        output.push_str(&esc::cursor_to(start_row + i, padding + 1));
        output.push_str(line);
    }

    let text_start_row = start_row + sad_mac.len() + 2;
    for (i, line) in messages.iter().enumerate() {
        let padding = (width - line.len()) / 2;
        output.push_str(&esc::cursor_to(text_start_row + i, padding + 1));
        output.push_str(line);
    }

    // Move cursor to bottom to be clean
    output.push_str(&esc::cursor_to(TERMINAL_HEIGHT, 1));
    output
}

/// Generate a placeholder frame for when waiting for a peer to call back
pub fn generate_waiting_for_peer_frame(peer_name: &str) -> Vec<String> {
    let raw_lines = vec![
        "      .---.      ".to_string(),
        "     /     \\     ".to_string(),
        "    |  (O)  |    ".to_string(),
        "     \\     /     ".to_string(),
        "      `---'      ".to_string(),
        "       _|_       ".to_string(),
        "      /   \\      ".to_string(),
        "".to_string(),
        "".to_string(),
        format!("When {} calls back,", peer_name),
        "the video call will start.".to_string(),
    ];

    let max_width = raw_lines.iter().map(|l| l.len()).max().unwrap_or(0);

    raw_lines
        .into_iter()
        .map(|line| {
            let padding = max_width.saturating_sub(line.len());
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), line, " ".repeat(right))
        })
        .collect()
}
