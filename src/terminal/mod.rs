//! VT100/VT220 terminal control sequences and UI rendering.
//!
//! This module provides:
//! - Terminal escape sequences and constants
//! - Chat buffer with scrollback support
//! - UI rendering (tab bar, input area, borders)
//! - Stream/video frame rendering

mod buffer;
mod render;
mod ui;

pub use buffer::ChatBuffer;
pub use render::{generate_waiting_for_peer_frame, render_stream};
pub use ui::{
    cleanup_split_screen, init_split_screen_with_tabs, max_input_length, redraw_input,
    redraw_tab_bar,
};

use crate::graphics::get_drcs_load_sequence;

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
        output.push_str(&get_drcs_load_sequence());
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
