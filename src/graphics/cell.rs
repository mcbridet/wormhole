//! Cell-based frame representation for efficient differential rendering.
//!
//! This module provides a structured representation of video frame "cells"
//! that separates content from rendering mode, enabling proper cell-by-cell
//! comparison for differential updates.

use super::{SHIFT_IN, SHIFT_OUT};

/// The character set mode for a cell
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharMode {
    /// Standard ASCII character
    Ascii,
    /// DEC Special Graphics or DRCS (accessed via G1, requires SHIFT_OUT)
    DecGraphics,
}

/// A single cell in a video frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character to display
    pub char: char,
    /// The character set mode
    pub mode: CharMode,
}

impl Cell {
    /// Create a new ASCII cell
    pub const fn ascii(c: char) -> Self {
        Self {
            char: c,
            mode: CharMode::Ascii,
        }
    }

    /// Create a new DEC graphics cell
    #[allow(dead_code)]
    pub const fn dec_graphics(c: char) -> Self {
        Self {
            char: c,
            mode: CharMode::DecGraphics,
        }
    }

    /// Create a space cell (ASCII)
    pub const fn space() -> Self {
        Self::ascii(' ')
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::space()
    }
}

/// A video frame represented as a grid of cells
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The cells, stored row-major
    pub rows: Vec<Vec<Cell>>,
}

impl Frame {
    /// Create a new empty frame
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Create a frame with the given dimensions filled with spaces
    #[allow(dead_code)]
    pub fn with_dimensions(width: usize, height: usize) -> Self {
        Self {
            rows: vec![vec![Cell::space(); width]; height],
        }
    }

    /// Get the height (number of rows)
    pub fn height(&self) -> usize {
        self.rows.len()
    }

    /// Get the width (columns in first row, or 0 if empty)
    pub fn width(&self) -> usize {
        self.rows.first().map(|r| r.len()).unwrap_or(0)
    }

    /// Convert the frame to a Vec<String> for network transmission or legacy compatibility.
    /// Each row becomes a string with embedded escape sequences.
    #[allow(dead_code)]
    pub fn to_strings(&self) -> Vec<String> {
        self.rows.iter().map(|row| row_to_string(row)).collect()
    }

    /// Parse a Vec<String> back into a Frame.
    /// This reconstructs cells from strings with embedded escape sequences.
    pub fn from_strings(lines: &[String]) -> Self {
        Self {
            rows: lines.iter().map(|line| parse_row(line)).collect(),
        }
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a row of cells to a string with escape sequences
#[allow(dead_code)]
fn row_to_string(row: &[Cell]) -> String {
    let mut output = String::with_capacity(row.len() + 10);
    let mut current_mode = CharMode::Ascii;

    for cell in row {
        if cell.mode != current_mode {
            match cell.mode {
                CharMode::Ascii => output.push_str(SHIFT_IN),
                CharMode::DecGraphics => output.push_str(SHIFT_OUT),
            }
            current_mode = cell.mode;
        }
        output.push(cell.char);
    }

    // Always end in ASCII mode
    if current_mode != CharMode::Ascii {
        output.push_str(SHIFT_IN);
    }

    output
}

/// Parse a string with escape sequences back into cells
fn parse_row(line: &str) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(line.len());
    let mut current_mode = CharMode::Ascii;

    for c in line.chars() {
        match c {
            '\x0E' => current_mode = CharMode::DecGraphics, // SHIFT_OUT
            '\x0F' => current_mode = CharMode::Ascii,       // SHIFT_IN
            _ => cells.push(Cell {
                char: c,
                mode: current_mode,
            }),
        }
    }

    cells
}

/// Render a frame with cell-by-cell differential updates.
///
/// Returns the escape sequences needed to update the terminal from `prev` to `current`,
/// positioning each changed cell individually.
///
/// # Arguments
/// * `current` - The new frame to render
/// * `prev` - The previous frame (if any)
/// * `start_row` - Terminal row (1-based) where the frame starts
/// * `start_col` - Terminal column (1-based) where the frame starts
/// * `cursor_to` - Function to generate cursor positioning escape sequence
pub fn render_frame_diff<F>(
    current: &Frame,
    prev: Option<&Frame>,
    start_row: usize,
    start_col: usize,
    cursor_to: F,
) -> String
where
    F: Fn(usize, usize) -> String,
{
    let mut output = String::new();

    // Check if we can do cell-level diffing
    let can_diff =
        prev.is_some_and(|p| p.height() == current.height() && p.width() == current.width());

    // Track current terminal mode to minimize escape sequences
    let mut terminal_mode = CharMode::Ascii;
    // Track if we need to reposition cursor
    let mut cursor_row: Option<usize> = None;
    let mut cursor_col: Option<usize> = None;

    for (row_idx, row) in current.rows.iter().enumerate() {
        let term_row = start_row + row_idx;

        if can_diff {
            let prev_row = &prev.unwrap().rows[row_idx];

            // Find changed cells in this row
            for (col_idx, cell) in row.iter().enumerate() {
                let prev_cell = &prev_row[col_idx];

                if cell != prev_cell {
                    let term_col = start_col + col_idx;

                    // Position cursor if needed
                    let need_position =
                        cursor_row != Some(term_row) || cursor_col != Some(term_col);

                    if need_position {
                        output.push_str(&cursor_to(term_row, term_col));
                    }

                    // Switch mode if needed
                    if cell.mode != terminal_mode {
                        match cell.mode {
                            CharMode::Ascii => output.push_str(SHIFT_IN),
                            CharMode::DecGraphics => output.push_str(SHIFT_OUT),
                        }
                        terminal_mode = cell.mode;
                    }

                    output.push(cell.char);
                    cursor_row = Some(term_row);
                    cursor_col = Some(term_col + 1); // Cursor advances after write
                }
            }
        } else {
            // Full redraw of this row
            output.push_str(&cursor_to(term_row, start_col));
            cursor_row = Some(term_row);
            cursor_col = Some(start_col);

            for cell in row {
                // Switch mode if needed
                if cell.mode != terminal_mode {
                    match cell.mode {
                        CharMode::Ascii => output.push_str(SHIFT_IN),
                        CharMode::DecGraphics => output.push_str(SHIFT_OUT),
                    }
                    terminal_mode = cell.mode;
                }
                output.push(cell.char);
                if let Some(ref mut col) = cursor_col {
                    *col += 1;
                }
            }
        }
    }

    // Return to ASCII mode at the end
    if terminal_mode != CharMode::Ascii {
        output.push_str(SHIFT_IN);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_creation() {
        let ascii = Cell::ascii('A');
        assert_eq!(ascii.char, 'A');
        assert_eq!(ascii.mode, CharMode::Ascii);

        let dec = Cell::dec_graphics('a');
        assert_eq!(dec.char, 'a');
        assert_eq!(dec.mode, CharMode::DecGraphics);
    }

    #[test]
    fn test_row_to_string() {
        let row = vec![
            Cell::ascii('H'),
            Cell::ascii('i'),
            Cell::dec_graphics('a'), // checkerboard
            Cell::dec_graphics('a'),
            Cell::ascii('!'),
        ];
        let s = row_to_string(&row);
        // Should be: "Hi" + SHIFT_OUT + "aa" + SHIFT_IN + "!"
        assert_eq!(s, "Hi\x0Eaa\x0F!");
    }

    #[test]
    fn test_parse_row() {
        let line = "Hi\x0Eaa\x0F!";
        let cells = parse_row(line);
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0], Cell::ascii('H'));
        assert_eq!(cells[1], Cell::ascii('i'));
        assert_eq!(cells[2], Cell::dec_graphics('a'));
        assert_eq!(cells[3], Cell::dec_graphics('a'));
        assert_eq!(cells[4], Cell::ascii('!'));
    }

    #[test]
    fn test_roundtrip() {
        let original = Frame {
            rows: vec![
                vec![Cell::ascii('A'), Cell::dec_graphics('a'), Cell::ascii('B')],
                vec![
                    Cell::dec_graphics('!'),
                    Cell::dec_graphics('"'),
                    Cell::ascii(' '),
                ],
            ],
        };

        let strings = original.to_strings();
        let reconstructed = Frame::from_strings(&strings);
        assert_eq!(original, reconstructed);
    }
}
