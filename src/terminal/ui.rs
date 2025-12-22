//! UI components: tab bar, input area, borders.

use super::Tab;
use super::esc;
use super::{
    CHAT_REGION_END, CHAT_REGION_START, INPUT_ROW_END, INPUT_ROW_START, INPUT_ROWS, TERMINAL_HEIGHT,
};
use crate::graphics::{DecGraphicsChar, ENTER_DEC_GRAPHICS, EXIT_DEC_GRAPHICS};

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
