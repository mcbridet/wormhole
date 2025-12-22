//! Stream/video frame rendering.

use super::buffer::visible_len;
use super::esc;
use super::{CALL_REGION_END, CALL_VISIBLE_LINES, CHAT_REGION_START};

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
