//! Sixel graphics support for VT340 terminals.
//!
//! Sixel is a bitmap graphics format supported by DEC VT240, VT241, VT330, VT340,
//! and many modern terminal emulators. It allows rendering actual bitmap images
//! in the terminal, providing much higher quality than character-based approaches.
//!
//! ## Sixel Format Overview
//!
//! Sixel data represents vertical strips of 6 pixels. Each character in the sixel
//! stream encodes which of the 6 vertical pixels are "on" for a given color.
//! The encoding is: character = (bitmap value) + 63 (0x3F).
//!
//! ## Escape Sequences
//!
//! - Enter Sixel mode: `ESC P <params> q <sixel-data> ESC \`
//! - DCS (Device Control String) introducer: `ESC P` or `0x90`
//! - ST (String Terminator): `ESC \` or `0x9C`

use image::{DynamicImage, GenericImageView, GrayImage, imageops::FilterType};

/// DCS (Device Control String) introducer for Sixel
pub const DCS: &str = "\x1bP";

/// ST (String Terminator)
pub const ST: &str = "\x1b\\";

/// Pixels per terminal row for sixel output
/// VT340: 10 scanlines per character cell
/// Using 18 gives good size while fitting in 22-row display area (396 pixels)
const PIXELS_PER_ROW: u32 = 18;

/// Configuration for sixel encoding
#[derive(Debug, Clone)]
pub struct SixelConfig {
    /// Number of grayscale levels (2-256)
    pub gray_levels: u8,
    /// Whether to use run-length encoding for compression
    pub use_rle: bool,
}

impl Default for SixelConfig {
    fn default() -> Self {
        Self {
            gray_levels: 8, // 8 shades - good balance for 38400 baud @ 3 FPS
            use_rle: true,
        }
    }
}

/// Encode a grayscale image as a sixel string.
///
/// # Arguments
/// * `image` - The grayscale image to encode
/// * `config` - Sixel encoding configuration
///
/// # Returns
/// A string containing the complete sixel sequence (DCS...ST)
pub fn encode_grayscale(image: &GrayImage, config: &SixelConfig) -> String {
    let (width, height) = image.dimensions();

    if width == 0 || height == 0 {
        return String::new();
    }

    let mut output = String::with_capacity((width * height / 2) as usize);

    // Start sixel sequence
    // Format: DCS P1 ; P2 ; P3 q
    // P1 = pixel aspect ratio (0 = default 2:1)
    // P2 = background select (0 = fill background with color 0, 1 = leave background)
    // P3 = horizontal grid size (0 = default)
    output.push_str(DCS);
    output.push_str("0;0;0q");

    // Set raster attributes: "width;height (pixels)
    // Format: "Pan;Pad;Ph;Pv where Pan/Pad are aspect ratio nums, Ph/Pv are pixel dimensions
    output.push_str(&format!("\"1;1;{};{}", width, height));

    // Define grayscale palette
    // Format: #Pc;2;Ph;Pl;Ps (Pc=color#, 2=HLS, Ph=hue, Pl=lightness, Ps=saturation)
    // For grayscale: hue=0, saturation=0, vary lightness from 0-100
    for i in 0..config.gray_levels {
        let lightness = (i as u32 * 100) / (config.gray_levels as u32 - 1);
        output.push_str(&format!("#{};2;0;{};0", i, lightness));
    }

    // Process image in bands of 6 rows (one sixel row)
    let num_bands = height.div_ceil(6);

    for band in 0..num_bands {
        let y_start = band * 6;

        // Track which colors have any pixels in this band
        let mut colors_used: Vec<bool> = vec![false; config.gray_levels as usize];

        // First pass: determine which colors are used in this band
        for x in 0..width {
            for bit in 0..6 {
                let y = y_start + bit;
                if y < height {
                    let pixel = image.get_pixel(x, y)[0];
                    let pixel_color = (pixel as u16 * (config.gray_levels - 1) as u16 / 255) as u8;
                    colors_used[pixel_color as usize] = true;
                }
            }
        }

        // Second pass: output sixel data for each used color
        let mut first_color_in_band = true;
        for color in 0..config.gray_levels {
            if !colors_used[color as usize] {
                continue;
            }

            // Graphics Carriage Return before each color except the first in band
            if !first_color_in_band {
                output.push('$');
            }
            first_color_in_band = false;

            // Select this color
            output.push('#');
            output.push_str(&color.to_string());

            let mut run_char: Option<char> = None;
            let mut run_length: u32 = 0;

            for x in 0..width {
                // Build the 6-bit value for this column
                let mut sixel_value: u8 = 0;

                for bit in 0..6 {
                    let y = y_start + bit;
                    if y < height {
                        let pixel = image.get_pixel(x, y)[0];
                        let pixel_color =
                            (pixel as u16 * (config.gray_levels - 1) as u16 / 255) as u8;

                        if pixel_color == color {
                            sixel_value |= 1 << bit;
                        }
                    }
                }

                // Convert to sixel character (add 63)
                let sixel_char = (sixel_value + 63) as char;

                if config.use_rle {
                    if Some(sixel_char) == run_char {
                        run_length += 1;
                    } else {
                        // Flush previous run
                        if let Some(ch) = run_char {
                            output.push_str(&encode_run(ch, run_length));
                        }
                        run_char = Some(sixel_char);
                        run_length = 1;
                    }
                } else {
                    output.push(sixel_char);
                }
            }

            // Flush final run for this color
            if config.use_rle
                && let Some(ch) = run_char
            {
                output.push_str(&encode_run(ch, run_length));
            }
        }

        // Graphics New Line (move to next band)
        // '-' = Graphics New Line
        if band < num_bands - 1 {
            output.push('-');
        }
    }

    // End sixel sequence
    output.push_str(ST);

    output
}

/// Encode a run of identical characters using RLE
fn encode_run(ch: char, count: u32) -> String {
    if count == 0 {
        String::new()
    } else if count == 1 {
        ch.to_string()
    } else if count == 2 {
        format!("{}{}", ch, ch)
    } else if count == 3 {
        format!("{}{}{}", ch, ch, ch)
    } else {
        // RLE format: !<count><char>
        format!("!{}{}", count, ch)
    }
}

/// Convert a DynamicImage to sixel format for terminal display.
///
/// # Arguments
/// * `image` - The source image
/// * `height_rows` - Target height in terminal rows
/// * `display_width` - Maximum width in characters/pixels
/// * `config` - Optional sixel configuration (uses default if None)
///
/// # Returns
/// A string containing the complete sixel sequence
pub fn image_to_sixel(
    image: &DynamicImage,
    height_rows: u32,
    display_width: usize,
    config: Option<&SixelConfig>,
) -> String {
    let default_config = SixelConfig::default();
    let config = config.unwrap_or(&default_config);

    // Calculate target dimensions in pixels
    // Each terminal row is approximately PIXELS_PER_ROW pixels
    let target_height = height_rows * PIXELS_PER_ROW;

    // VT340 character cell is approximately 10 pixels wide
    // So display_width columns = display_width * 10 pixels
    const PIXELS_PER_COL: u32 = 10;
    let max_width_pixels = (display_width.saturating_sub(4) as u32) * PIXELS_PER_COL;

    // Calculate aspect-correct width
    let (img_w, img_h) = image.dimensions();
    let aspect = img_w as f32 / img_h as f32;

    // For wide displays, ensure at least 16:9
    let aspect = if display_width > 100 {
        aspect.max(1.77)
    } else {
        aspect
    };

    let ideal_width = (target_height as f32 * aspect) as u32;
    let target_width = ideal_width.min(max_width_pixels);

    // Resize image to target dimensions
    let resized = image.resize_to_fill(target_width, target_height, FilterType::Triangle);

    // Convert to grayscale and enhance contrast
    let gray = resized.to_luma8();
    let enhanced = enhance_contrast(&gray);

    encode_grayscale(&enhanced, config)
}

/// Apply contrast enhancement to a grayscale image
/// Uses histogram stretching + S-curve for extra punch
fn enhance_contrast(image: &GrayImage) -> GrayImage {
    // Find min and max pixel values (use percentiles to ignore outliers)
    let mut histogram = [0u32; 256];
    let total_pixels = image.width() * image.height();

    for pixel in image.pixels() {
        histogram[pixel[0] as usize] += 1;
    }

    // Find 2nd and 98th percentile for robust stretching
    let low_threshold = total_pixels / 50; // 2%
    let high_threshold = total_pixels - (total_pixels / 50); // 98%

    let mut count = 0u32;
    let mut min_val: u8 = 0;
    let mut max_val: u8 = 255;

    for (i, &freq) in histogram.iter().enumerate() {
        count += freq;
        if count >= low_threshold && min_val == 0 {
            min_val = i as u8;
        }
        if count >= high_threshold {
            max_val = i as u8;
            break;
        }
    }

    // Avoid division by zero and extreme stretching of noise
    if max_val <= min_val {
        max_val = min_val + 1;
    }

    // Prevent over-stretching of dark images
    if max_val < 100 {
        max_val = 100;
    }

    let range = (max_val - min_val) as f32;
    let mut result = image.clone();

    for pixel in result.pixels_mut() {
        let val = pixel[0];
        let clamped = val.max(min_val).min(max_val);
        let normalized = (clamped - min_val) as f32 / range;

        // S-curve: 3x^2 - 2x^3 (smoothstep)
        let curved = normalized * normalized * (3.0 - 2.0 * normalized);

        pixel[0] = (curved * 255.0) as u8;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GrayImage;

    #[test]
    fn test_sixel_config_default() {
        let config = SixelConfig::default();
        assert_eq!(config.gray_levels, 8);
        assert!(config.use_rle);
    }

    #[test]
    fn test_encode_run() {
        assert_eq!(encode_run('A', 0), "");
        assert_eq!(encode_run('A', 1), "A");
        assert_eq!(encode_run('A', 2), "AA");
        assert_eq!(encode_run('A', 3), "AAA");
        assert_eq!(encode_run('A', 4), "!4A");
        assert_eq!(encode_run('A', 100), "!100A");
    }

    #[test]
    fn test_encode_small_image() {
        // Create a simple 6x6 grayscale test image
        let mut img = GrayImage::new(6, 6);
        for y in 0..6 {
            for x in 0..6 {
                let brightness = ((x + y) * 20) as u8;
                img.put_pixel(x, y, image::Luma([brightness]));
            }
        }

        let config = SixelConfig {
            gray_levels: 4,
            use_rle: false,
        };

        let result = encode_grayscale(&img, &config);

        // Verify it starts with DCS and ends with ST
        assert!(result.starts_with(DCS));
        assert!(result.ends_with(ST));

        // Verify palette definitions exist
        assert!(result.contains("#0"));
        assert!(result.contains("#1"));
        assert!(result.contains("#2"));
        assert!(result.contains("#3"));
    }

    #[test]
    fn test_empty_image() {
        let img = GrayImage::new(0, 0);
        let config = SixelConfig::default();
        let result = encode_grayscale(&img, &config);
        assert!(result.is_empty());
    }
}
