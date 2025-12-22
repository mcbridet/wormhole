//! VT100/VT120 terminal control sequences and UI rendering.

use crate::dec_graphics::{DecGraphicsChar, ENTER_DEC_GRAPHICS, EXIT_DEC_GRAPHICS};
use std::collections::VecDeque;

/// VT120 terminal dimensions (80x24 is standard)
pub const TERMINAL_WIDTH: usize = 80;
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
/// Each row has: left border (1) + content (76) + space (1) + right border (1) = 79 visible
/// First row also has prompt taking some space
pub const INPUT_CONTENT_WIDTH: usize = TERMINAL_WIDTH - 4; // 76 chars per row (excluding borders and padding)

/// Maximum scrollback buffer size
pub const MAX_SCROLLBACK: usize = 10_000;

/// Tab identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Chat = 0,
    Gemini = 1,
    Call = 2,
}

impl Tab {
    pub fn next(self, gemini_available: bool, call_active: bool) -> Self {
        match self {
            Tab::Chat => {
                if gemini_available {
                    Tab::Gemini
                } else if call_active {
                    Tab::Call
                } else {
                    Tab::Chat
                }
            }
            Tab::Gemini => {
                if call_active {
                    Tab::Call
                } else {
                    Tab::Chat
                }
            }
            Tab::Call => Tab::Chat,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Chat => "Chat",
            Tab::Gemini => "AI",
            Tab::Call => "Call",
        }
    }
}

/// Chat buffer with scrollback support
pub struct ChatBuffer {
    /// All messages in the buffer
    lines: VecDeque<String>,
    /// Current scroll offset (0 = viewing most recent, >0 = scrolled up)
    scroll_offset: usize,
}

impl ChatBuffer {
    /// Create a new empty chat buffer
    pub fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(MAX_SCROLLBACK),
            scroll_offset: 0,
        }
    }
    
    /// Add a message to the buffer, wrapping if necessary
    pub fn push(&mut self, message: String) {
        if message.is_empty() {
            self.push_raw(String::new());
            return;
        }

        let max_len = TERMINAL_WIDTH - 4; // "│ " on left, " │" on right
        
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
                        first_word = true;
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
        let max_len = TERMINAL_WIDTH - 4;
        let truncated = if content.len() > max_len {
            content[..max_len].to_string()
        } else {
            content.to_string()
        };
        
        if let Some(last) = self.lines.back_mut() {
            *last = truncated;
        }
    }
    
    /// Remove the last line from the buffer (useful for replacing streaming placeholder)
    pub fn pop_last(&mut self) {
        self.lines.pop_back();
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
    
    /// Check if we're scrolled up (not viewing latest)
    pub fn is_scrolled(&self) -> bool {
        self.scroll_offset > 0
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
    
    /// Render the entire chat area
    pub fn render(&self) -> String {
        use DecGraphicsChar::VerticalLine;
        
        let mut output = String::new();
        let visible = self.visible_lines();
        let max_len = TERMINAL_WIDTH - 4;
        
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
                for _ in line.len()..max_len {
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
    
    /// Render just the scroll indicator (if scrolled up)
    pub fn render_scroll_indicator(&self) -> String {
        if self.scroll_offset > 0 {
            let mut output = String::new();
            output.push_str(esc::SAVE_CURSOR);
            // Show indicator at top-right of chat area
            output.push_str(&esc::cursor_to(CHAT_REGION_START, TERMINAL_WIDTH - 10));
            output.push_str(&format!("[+{}]", self.scroll_offset));
            output.push_str(esc::RESTORE_CURSOR);
            output
        } else {
            String::new()
        }
    }
}

/// Render a stream frame to the content area
pub fn render_stream(sender: &str, lines: &[String]) -> String {
    let mut output = String::new();
    
    // Clear content area first (fill with spaces)
    // Reserve row 23 for status message
    for row in CHAT_REGION_START..CALL_REGION_END {
        output.push_str(&esc::cursor_to(row, 2));
        output.push_str(&" ".repeat(TERMINAL_WIDTH - 2));
    }
    
    // Calculate centering
    let frame_height = lines.len();
    let frame_width = if frame_height > 0 { lines[0].len() } else { 0 };
    
    let start_row = CHAT_REGION_START + (CALL_VISIBLE_LINES.saturating_sub(frame_height)) / 2;
    let start_col = (TERMINAL_WIDTH.saturating_sub(frame_width)) / 2 + 1; // 1-based
    
    // Draw sender name at top of content area
    // output.push_str(&esc::cursor_to(CHAT_REGION_START, 2));
    // output.push_str(&format!("Streaming: {}", sender));
    
    // Draw frame
    for (i, line) in lines.iter().enumerate() {
        let row = start_row + i;
        if row >= CALL_REGION_END { break; }
        
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
    
    /// Bold/bright mode
    pub const BOLD: &str = "\x1b[1m";
    
    /// Reverse video
    pub const REVERSE: &str = "\x1b[7m";
    
    /// Save cursor position
    pub const SAVE_CURSOR: &str = "\x1b7";
    
    /// Restore cursor position
    pub const RESTORE_CURSOR: &str = "\x1b8";
    
    /// Clear to end of line
    pub const CLEAR_EOL: &str = "\x1b[K";
    
    /// Move cursor to specific position (1-indexed)
    pub fn cursor_to(row: usize, col: usize) -> String {
        format!("\x1b[{};{}H", row, col)
    }
    
    /// Set scroll region (1-indexed, inclusive)
    pub fn set_scroll_region(top: usize, bottom: usize) -> String {
        format!("\x1b[{};{}r", top, bottom)
    }
    
    /// Reset scroll region to full screen
    pub fn reset_scroll_region() -> String {
        "\x1b[r".to_string()
    }
}

/// Draw a horizontal line with optional left/right connectors
fn draw_horizontal_line(left: DecGraphicsChar, right: DecGraphicsChar) -> String {
    use DecGraphicsChar::HorizontalLine;
    
    let mut output = String::new();
    output.push_str(ENTER_DEC_GRAPHICS);
    output.push(left.as_dec_char());
    for _ in 0..TERMINAL_WIDTH - 2 {
        output.push(HorizontalLine.as_dec_char());
    }
    output.push(right.as_dec_char());
    output.push_str(EXIT_DEC_GRAPHICS);
    output
}

/// Draw the top border with tab indicators
fn draw_tab_bar(active_tab: Tab, gemini_available: bool, active_call: Option<&str>) -> String {
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
    output.push_str(&draw_tab("Chat", active_tab == Tab::Chat, next_tab == Tab::Chat));
    
    // AI Tab (if available)
    if gemini_available {
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(HorizontalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);
        
        output.push_str(&draw_tab("AI", active_tab == Tab::Gemini, next_tab == Tab::Gemini));
    }

    // Call Tab (if active)
    if let Some(peer_name) = active_call {
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(HorizontalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);
        
        let label = format!("Call ({})", peer_name);
        output.push_str(&draw_tab(&label, active_tab == Tab::Call, next_tab == Tab::Call));
    }
    
    // Hints: ^Refresh / ^Clear
    let hints = " ^Refresh / ^Clear ";
    
    // Calculate used length
    let mut visible_len = 1; // Corner
    
    // Chat
    visible_len += if active_tab == Tab::Chat { 6 } else if next_tab == Tab::Chat { 12 } else { 6 };
    
    // AI
    if gemini_available {
        visible_len += 1; // Separator
        visible_len += if active_tab == Tab::Gemini { 4 } else if next_tab == Tab::Gemini { 10 } else { 4 };
    }

    // Call
    if let Some(peer_name) = active_call {
        visible_len += 1; // Separator
        let label_len = 7 + peer_name.len();
        let tab_len = label_len + 2;
        let next_len = label_len + 8;
        visible_len += if active_tab == Tab::Call { tab_len } else if next_tab == Tab::Call { next_len } else { tab_len };
    }
    
    visible_len += hints.len();
    visible_len += 1; // Right corner
    
    // Fill with horizontal line
    let remaining = if visible_len < TERMINAL_WIDTH {
        TERMINAL_WIDTH - visible_len
    } else {
        0
    };
    
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
pub fn redraw_tab_bar(active_tab: Tab, gemini_available: bool, active_call: Option<&str>) -> String {
    let mut output = String::new();
    output.push_str(esc::SAVE_CURSOR);
    output.push_str(&draw_tab_bar(active_tab, gemini_available, active_call));
    output.push_str(esc::RESTORE_CURSOR);
    output
}

/// Calculate the maximum input length based on prompt size
pub fn max_input_length(client_name: &str) -> usize {
    let prompt = format!("[{}] ", client_name);
    let prompt_len = prompt.len();
    
    // First row: content width minus prompt
    let first_row_capacity = INPUT_CONTENT_WIDTH - prompt_len;
    // Subsequent rows: full content width
    let other_rows_capacity = INPUT_CONTENT_WIDTH * (INPUT_ROWS - 1);
    
    first_row_capacity + other_rows_capacity
}

/// Initialize the split-screen UI with borders
/// Returns the escape sequence to set up the terminal
pub fn init_split_screen(client_name: &str) -> String {
    init_split_screen_with_tabs(client_name, Tab::Chat, false, None, None)
}

/// Initialize the split-screen UI with borders and tab support
pub fn init_split_screen_with_tabs(client_name: &str, active_tab: Tab, gemini_available: bool, active_call: Option<&str>, call_status: Option<&str>) -> String {
    use DecGraphicsChar::*;
    
    let prompt = format!("[{}] ", client_name);
    let mut output = String::new();
    
    // Clear screen
    output.push_str(esc::CLEAR_SCREEN);
    output.push_str(esc::CURSOR_HOME);
    
    // Row 1: Top border with tabs
    output.push_str(&draw_tab_bar(active_tab, gemini_available, active_call));
    
    if active_tab == Tab::Call {
        // Draw full box for Call (no split)
        // Rows 2-23: Left and right borders
        for row in 2..=23 {
            output.push_str(&esc::cursor_to(row, 1));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push_str(&esc::cursor_to(row, TERMINAL_WIDTH));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }
        
        // Row 24: Bottom border
        output.push_str(&esc::cursor_to(24, 1));
        output.push_str(&draw_horizontal_line(LowerLeftCorner, LowerRightCorner));
        
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
            output.push_str(&esc::cursor_to(row, TERMINAL_WIDTH));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }
        
        // Row 21: Separator ├────────────────────┤
        output.push_str(&esc::cursor_to(21, 1));
        output.push_str(&draw_horizontal_line(LeftTee, RightTee));
        
        // Rows 21-23: Input area borders
        for row in INPUT_ROW_START..=INPUT_ROW_END {
            output.push_str(&esc::cursor_to(row, 1));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
            output.push_str(&esc::cursor_to(row, TERMINAL_WIDTH));
            output.push_str(ENTER_DEC_GRAPHICS);
            output.push(VerticalLine.as_dec_char());
            output.push_str(EXIT_DEC_GRAPHICS);
        }
        
        // Draw prompt on first input row
        output.push_str(&esc::cursor_to(INPUT_ROW_START, 2));
        output.push_str(&prompt);
        
        // Row 24: Bottom border └────────────────────┘
        output.push_str(&esc::cursor_to(24, 1));
        output.push_str(&draw_horizontal_line(LowerLeftCorner, LowerRightCorner));
        
        // No scroll region - we manage scrolling ourselves via ChatBuffer
        
        // Position cursor at input area (after prompt)
        output.push_str(&esc::cursor_to(INPUT_ROW_START, 2 + prompt.len()));
        
        // Show cursor
        output.push_str(esc::CURSOR_SHOW);
    }
    
    output
}

/// Print a message to the chat area - DEPRECATED, use ChatBuffer instead
/// Kept for compatibility during transition
pub fn print_to_chat(_message: &str) -> String {
    // This function is deprecated - use ChatBuffer.push() and ChatBuffer.render() instead
    String::new()
}

/// Redraw the input line with current buffer content
pub fn redraw_input(client_name: &str, buffer: &str) -> String {
    use DecGraphicsChar::VerticalLine;
    
    let prompt = format!("[{}] ", client_name);
    let prompt_len = prompt.len();
    let mut output = String::new();
    
    // Calculate capacity for each row
    let first_row_capacity = INPUT_CONTENT_WIDTH - prompt_len;
    
    // Split buffer into rows
    let mut remaining = buffer;
    let mut row_contents: Vec<&str> = Vec::new();
    
    // First row gets less space due to prompt
    if remaining.len() <= first_row_capacity {
        row_contents.push(remaining);
        remaining = "";
    } else {
        row_contents.push(&remaining[..first_row_capacity]);
        remaining = &remaining[first_row_capacity..];
    }
    
    // Subsequent rows get full width
    for _ in 1..INPUT_ROWS {
        if remaining.is_empty() {
            row_contents.push("");
        } else if remaining.len() <= INPUT_CONTENT_WIDTH {
            row_contents.push(remaining);
            remaining = "";
        } else {
            row_contents.push(&remaining[..INPUT_CONTENT_WIDTH]);
            remaining = &remaining[INPUT_CONTENT_WIDTH..];
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
            for _ in content.len()..first_row_capacity {
                output.push(' ');
            }
        } else {
            output.push_str(content);
            // Pad to clear old content
            for _ in content.len()..INPUT_CONTENT_WIDTH {
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
    let total_len = buffer.len();
    let (cursor_row, cursor_col) = if total_len <= first_row_capacity {
        // Cursor on first row
        (INPUT_ROW_START, 2 + prompt_len + total_len)
    } else {
        // Calculate which row and column
        let chars_after_first = total_len - first_row_capacity;
        let row_index = 1 + chars_after_first / INPUT_CONTENT_WIDTH;
        let col_in_row = chars_after_first % INPUT_CONTENT_WIDTH;
        (INPUT_ROW_START + row_index, 2 + col_in_row)
    };
    
    output.push_str(&esc::cursor_to(cursor_row, cursor_col));
    
    output
}

/// Cleanup: reset scroll region before exit
pub fn cleanup_split_screen() -> String {
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
        "Please check logs or restart the device."
    ];
    
    let total_lines = sad_mac.len() + 2 + messages.len(); // +2 for spacing
    let start_row = (TERMINAL_HEIGHT - total_lines) / 2;
    
    for (i, line) in sad_mac.iter().enumerate() {
        let padding = (TERMINAL_WIDTH - line.len()) / 2;
        output.push_str(&esc::cursor_to(start_row + i, padding + 1));
        output.push_str(line);
    }
    
    let text_start_row = start_row + sad_mac.len() + 2;
    for (i, line) in messages.iter().enumerate() {
        let padding = (TERMINAL_WIDTH - line.len()) / 2;
        output.push_str(&esc::cursor_to(text_start_row + i, padding + 1));
        output.push_str(line);
    }
    
    // Move cursor to bottom to be clean
    output.push_str(&esc::cursor_to(TERMINAL_HEIGHT, 1));
    output
}

/// Draw a centered box with text content
pub fn draw_centered_box(lines: &[&str]) -> String {
    use DecGraphicsChar::*;
    
    // Calculate box dimensions
    let max_line_len = lines.iter().map(|l| l.len()).max().unwrap_or(0);
    let box_width = max_line_len + 4; // 2 chars padding on each side
    let box_height = lines.len() + 2; // top and bottom borders
    
    // Calculate starting position for centering
    let start_col = (TERMINAL_WIDTH.saturating_sub(box_width)) / 2 + 1;
    let start_row = (TERMINAL_HEIGHT.saturating_sub(box_height)) / 2 + 1;
    
    let mut output = String::new();
    
    // Clear screen and hide cursor
    output.push_str(esc::CLEAR_SCREEN);
    output.push_str(esc::CURSOR_HOME);
    output.push_str(esc::CURSOR_HIDE);
    
    // Draw top border
    output.push_str(&esc::cursor_to(start_row, start_col));
    output.push_str(ENTER_DEC_GRAPHICS);
    output.push(UpperLeftCorner.as_dec_char());
    for _ in 0..box_width - 2 {
        output.push(HorizontalLine.as_dec_char());
    }
    output.push(UpperRightCorner.as_dec_char());
    output.push_str(EXIT_DEC_GRAPHICS);
    
    // Draw content rows
    for (i, line) in lines.iter().enumerate() {
        output.push_str(&esc::cursor_to(start_row + 1 + i, start_col));
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(VerticalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);
        
        // Center the text within the box
        let padding_total = box_width - 2 - line.len();
        let padding_left = padding_total / 2;
        let padding_right = padding_total - padding_left;
        
        for _ in 0..padding_left {
            output.push(' ');
        }
        output.push_str(line);
        for _ in 0..padding_right {
            output.push(' ');
        }
        
        output.push_str(ENTER_DEC_GRAPHICS);
        output.push(VerticalLine.as_dec_char());
        output.push_str(EXIT_DEC_GRAPHICS);
    }
    
    // Draw bottom border
    output.push_str(&esc::cursor_to(start_row + box_height - 1, start_col));
    output.push_str(ENTER_DEC_GRAPHICS);
    output.push(LowerLeftCorner.as_dec_char());
    for _ in 0..box_width - 2 {
        output.push(HorizontalLine.as_dec_char());
    }
    output.push(LowerRightCorner.as_dec_char());
    output.push_str(EXIT_DEC_GRAPHICS);
    
    output
}

/// Draw the application splash screen
pub fn draw_splash_screen(name: &str, version: &str, author: &str) -> String {
    let version_line = format!("Version {}", version);
    let lines: Vec<&str> = vec![
        "",
        name,
        "",
        &version_line,
        "",
        author,
        "",
    ];
    
    draw_centered_box(&lines)
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
    
    raw_lines.into_iter().map(|line| {
        let padding = max_width.saturating_sub(line.len());
        let left = padding / 2;
        let right = padding - left;
        format!("{}{}{}", " ".repeat(left), line, " ".repeat(right))
    }).collect()
}
