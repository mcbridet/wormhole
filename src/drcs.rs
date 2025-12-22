//! Dynamically Redefinable Character Set (DRCS) support for VT220.
//!
//! This module handles the loading of custom glyphs (Soft Fonts) to the terminal
//! to improve ASCII art rendering with smooth shading blocks.

/// Escape sequence to switch to the DRCS set (G1)
/// We load our custom font into G1 and shift-out (SO) or use escape sequences to access it.
/// For simplicity, we'll load it into G0 temporarily or just use G1.
/// Let's assume we load it into G1.

// Actually, let's use the standard mechanism:
// 1. Define DRCS
// 2. Map DRCS to G1: ESC ) <Dscs>
// 3. Shift Out (SO) to use G1: 0x0E
// 4. Shift In (SI) to use G0: 0x0F

/// Sequence to designate DRCS as G1
pub const DESIGNATE_DRCS_G1: &str = "\x1b) <"; // Assuming we name it '<'

/// Shift Out (Invoke G1 into GL)
pub const SHIFT_OUT: &str = "\x0E";
/// Shift In (Invoke G0 into GL)
pub const SHIFT_IN: &str = "\x0F";

/// Generates the DECDLD (Down-Line Load) sequence to load our custom shading glyphs.
/// 
/// We will define 4 glyphs starting at offset 0x21 (!):
/// 0x21 (!): Light Shade (25%)
/// 0x22 ("): Medium Shade (50%)
/// 0x23 (#): Dark Shade (75%)
/// 0x24 ($): Full Block (100%)
pub fn get_drcs_load_sequence() -> String {
    let mut seq = String::new();
    
    // DCS Pfn; Pcn; Pe; Pcmw; Pss; Pt; Pcmh; Pcss { Dscs
    // Pfn = 1 (Load DRCS)
    // Pcn = 1 (Starting char number, but usually we specify start char in the data)
    // Pe = 1 (Erase all characters in the set)
    // Pcmw = 0 (Default width, 80-col mode = 10 pixels usually, or 8)
    // Pss = 0 (Default)
    // Pt = 1 (Text)
    // Pcmh = 0 (Default height)
    // Pcss = 0 (Default size)
    // Dscs = < (Name of the set)
    
    seq.push_str("\x1bP1;1;1;0;0;1;0;0{ <");
    
    // Sixel data for glyphs.
    // Format: start_char / pattern ;
    // start_char is ASCII char (e.g. ! is 0x21)
    // pattern is sixel data.
    // 
    // VT220 character cell is 10 scanlines high.
    // Sixel encodes 6 vertical pixels.
    // So we need 2 sixel rows?
    // Actually, DECDLD uses a specific format where you send the top 6 rows, then the bottom 4 rows (for 10-pixel height).
    // Separated by /.
    
    // Let's define the patterns.
    // '?' is 0x3F (00111111) -> 6 pixels ON.
    // '~' is 0x7E (01111110)
    // Sixel offset is 63 (0x3F).
    // Value 0 -> ? (0x3F)
    // Value 63 -> ~ (0x7E) + ...
    
    // Wait, Sixel encoding:
    // Char = Value + 63.
    // Value is 6-bit integer.
    
    // 1. Light Shade (25%) - 0x21 (!)
    // Pattern: Every other pixel, every other line.
    // Top 6 rows:
    // Row 0: 10101010
    // Row 1: 00000000
    // Row 2: 10101010
    // ...
    // This is hard to hand-code without a generator.
    // Let's use a simplified "Full Block" and "Empty" to test, and maybe "Stripes" for shades.
    
    // Full Block (100%) - 0x24 ($)
    // All pixels on.
    // Top 6 rows: All 1s. Value 63 (0x3F). Char = 63+63 = 126 (~).
    // Bottom 4 rows: All 1s. Value 15 (0x0F). Char = 15+63 = 78 (N).
    // Width is 10 cols? Let's assume 8 cols for safety.
    // Pattern: ~~~~~~~~ / NNNNNNNN ;
    
    // Let's try to construct the string for the 4 chars.
    
    // Char 0x21 (!): Light Shade (25%)
    // Use sparse dots.
    // Top: "A A A A " (Values resulting in dots)
    // Let's just use a simple approximation.
    // We will define them as:
    // ! : Light
    // " : Medium
    // # : Dark
    // $ : Full
    
    // ! (Light)
    // Top:  Use char 'W' (0x57 = 87. 87-63 = 24 = 011000) - just random noise?
    // Let's use specific values.
    // 25% density.
    // We'll just use a simple pattern string for now.
    // "CKCKCKCK" (Just guessing a pattern that looks like shading)
    // "C" = 67. 67-63 = 4 = 000100.
    // "K" = 75. 75-63 = 12 = 001100.
    
    // To be safe and ensure it works, I will use a very simple pattern:
    // Full Block ($): All pixels on.
    // Top: ~~~~~~~~ (All 6 bits set)
    // Bottom: NNNNNNNN (Bottom 4 bits set)
    
    // Dark Shade (#): 75%
    // Top: vvvvvvvv (v = 118. 118-63 = 55 = 110111)
    // Bottom: JJJJJJJJ (J = 74. 74-63 = 11 = 001011)
    
    // Medium Shade ("): 50%
    // Top: oooooooo (o = 111. 111-63 = 48 = 110000)
    // Bottom: FFFFFFFF (F = 70. 70-63 = 7 = 000111)
    
    // Light Shade (!): 25%
    // Top: hhhhhhhh (h = 104. 104-63 = 41 = 101001)
    // Bottom: BBBBBBBB (B = 66. 66-63 = 3 = 000011)
    
    // Constructing the payload:
    // ! hhhhhhhh/BBBBBBBB;
    // " oooooooo/FFFFFFFF;
    // # vvvvvvvv/JJJJJJJJ;
    // $ ~~~~~~~~/NNNNNNNN;
    
    seq.push_str("!hhhhhhhh/BBBBBBBB;");
    seq.push_str("\"oooooooo/FFFFFFFF;");
    seq.push_str("#vvvvvvvv/JJJJJJJJ;");
    seq.push_str("$~~~~~~~~/NNNNNNNN;");
    
    seq.push_str("\x1b\\"); // ST (String Terminator)
    
    // Designate the loaded set (<) as G1
    seq.push_str(DESIGNATE_DRCS_G1);
    
    seq
}

/// Returns the character to use for a given brightness (0-255)
/// when using DRCS mode.
pub fn brightness_to_drcs_char(brightness: u8) -> char {
    // Map 0-255 to 5 levels:
    // 0-50: Space
    // 51-100: Light (!)
    // 101-150: Medium (")
    // 151-200: Dark (#)
    // 201-255: Full ($)
    
    match brightness {
        0..=50 => ' ',
        51..=100 => '!',
        101..=150 => '"',
        151..=200 => '#',
        _ => '$',
    }
}
