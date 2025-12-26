//! Stream/video frame rendering.

use super::esc;
use super::{CALL_REGION_END, CALL_VISIBLE_LINES, CHAT_REGION_START};
use crate::graphics::{Frame, render_frame_diff};

/// Check if content is sixel data (starts with DCS = ESC P)
fn is_sixel_data(lines: &[String]) -> bool {
    lines.len() == 1 && lines[0].starts_with("\x1bP")
}

/// Render a stream frame to the content area using cell-based differential rendering.
///
/// This function parses the lines into structured cells, compares them cell-by-cell
/// with the previous frame, and emits minimal escape sequences to update only
/// the changed cells. This works correctly with hybrid ASCII/DEC graphics.
///
/// For sixel graphics (VT340), the content is rendered as a bitmap block with
/// cursor positioning, bypassing cell-based diffing.
pub fn render_stream(
    _sender: &str,
    lines: &[String],
    prev_frame: Option<&Frame>,
    width: usize,
) -> (String, Frame) {
    // Check if this is sixel data
    if is_sixel_data(lines) {
        return render_sixel_stream(&lines[0], prev_frame, width);
    }

    // Parse lines into structured cells
    let current_frame = Frame::from_strings(lines);

    // Calculate centering
    let frame_height = current_frame.height();
    let frame_width = current_frame.width();

    // Use integer division for centering, but ensure we don't start before CHAT_REGION_START
    let start_row = CHAT_REGION_START + (CALL_VISIBLE_LINES.saturating_sub(frame_height)) / 2;
    let start_col = (width.saturating_sub(frame_width)) / 2 + 1; // 1-based

    // Check if centering has changed (dimensions mismatch)
    let prev_for_diff =
        prev_frame.filter(|prev| prev.height() == frame_height && prev.width() == frame_width);

    // Render using cell-based diffing, limiting to visible region
    let output = render_frame_diff_limited(
        &current_frame,
        prev_for_diff,
        start_row,
        start_col,
        CALL_REGION_END,
    );

    (output, current_frame)
}

/// Render frame diff with row limit
fn render_frame_diff_limited(
    current: &Frame,
    prev: Option<&Frame>,
    start_row: usize,
    start_col: usize,
    max_row: usize,
) -> String {
    // Create a truncated frame if needed
    let visible_rows = max_row.saturating_sub(start_row);
    if current.height() <= visible_rows {
        // All rows visible, use standard diff
        render_frame_diff(current, prev, start_row, start_col, esc::cursor_to)
    } else {
        // Truncate frame to visible region
        let truncated = Frame {
            rows: current.rows[..visible_rows].to_vec(),
        };
        let prev_truncated = prev.map(|p| Frame {
            rows: p.rows[..visible_rows.min(p.height())].to_vec(),
        });
        render_frame_diff(
            &truncated,
            prev_truncated.as_ref(),
            start_row,
            start_col,
            esc::cursor_to,
        )
    }
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

/// Render sixel graphics data with cursor positioning.
///
/// Sixel is a bitmap format that can't use cell-based diffing.
/// We position the cursor at the top-left of the display area and output
/// the sixel data directly. The terminal handles the bitmap rendering.
///
/// For frame-level diffing, we store a hash of the sixel data in a special
/// "marker" Frame that can be compared for equality.
fn render_sixel_stream(
    sixel_data: &str,
    prev_frame: Option<&Frame>,
    _width: usize,
) -> (String, Frame) {
    // Create a marker frame for this sixel data
    // We use a special frame with a single cell containing a hash-like marker
    // This allows us to detect if the sixel content has changed
    let marker = create_sixel_marker_frame(sixel_data);

    // Check if we can skip rendering (same sixel data as before)
    if let Some(prev) = prev_frame
        && *prev == marker
    {
        // Content unchanged, skip rendering
        return (String::new(), marker);
    }

    // Calculate positioning for sixel image
    // Row 2 is where content starts (row 1 is the tab bar)
    // Column 2 is where content starts (column 1 is the left border)
    let start_row = CHAT_REGION_START;
    let start_col = 2; // Start after left border

    // Build output: position cursor, then sixel data
    let mut output = String::with_capacity(sixel_data.len() + 20);
    output.push_str(&esc::cursor_to(start_row, start_col));
    output.push_str(sixel_data);

    (output, marker)
}

/// Create a marker Frame for sixel data comparison.
///
/// Since we can't parse sixel into cells, we create a special marker frame
/// that stores a simple hash of the sixel data. Two frames with identical
/// sixel data will compare equal.
fn create_sixel_marker_frame(sixel_data: &str) -> Frame {
    use crate::graphics::Cell;

    // Create a simple hash by sampling characters from the sixel data
    // This is fast and sufficient for detecting changes
    let len = sixel_data.len();
    let sample_size = 8.min(len);

    let mut cells = Vec::with_capacity(sample_size + 2);

    // Add a marker prefix to distinguish from regular frames
    cells.push(Cell::ascii('\x1b')); // ESC
    cells.push(Cell::ascii('P')); // 'P' for sixel DCS

    // Sample characters from the sixel data for comparison
    if len > 0 {
        let step = len / sample_size.max(1);
        for i in 0..sample_size {
            let idx = (i * step).min(len - 1);
            if let Some(ch) = sixel_data.chars().nth(idx) {
                cells.push(Cell::ascii(ch));
            }
        }
    }

    // Also include length as part of the "hash"
    for digit in len.to_string().chars() {
        cells.push(Cell::ascii(digit));
    }

    Frame { rows: vec![cells] }
}
