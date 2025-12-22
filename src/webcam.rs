//! Webcam capture and ASCII art conversion for VT100/VT120 terminals.

use image::{imageops::FilterType, DynamicImage};
use nokhwa::{
    pixel_format::RgbFormat,
    utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution},
    Camera,
};

/// ASCII characters ordered by visual density (light to dark)
const ASCII_RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// Terminal dimensions for the image (leaving room for borders)
/// For proper aspect ratio with ~2:1 character cells:
/// - Each character is roughly 2x tall as wide
/// - So 16 rows = 32 "visual units" tall
/// - We use 65 chars width for a ~2:1 aspect ratio (wider than 16:9)
/// - We center the 65-char image in the 76-char display width
const IMAGE_WIDTH: u32 = 65;
/// Height in terminal rows
const IMAGE_HEIGHT: u32 = 16;
/// Height in terminal rows for Call mode
const CALL_IMAGE_HEIGHT: u32 = 20;
/// Display width (for centering)
const DISPLAY_WIDTH: usize = 76;

/// Error type for webcam operations
#[derive(Debug)]
pub enum WebcamError {
    NokhwaError(nokhwa::NokhwaError),
    NotConfigured,
    InvalidDevice(String),
}

impl std::fmt::Display for WebcamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebcamError::NokhwaError(e) => write!(f, "Webcam error: {}", e),
            WebcamError::NotConfigured => write!(f, "Webcam not configured, sorry!"),
            WebcamError::InvalidDevice(s) => write!(f, "Invalid webcam device: {}", s),
        }
    }
}

impl From<nokhwa::NokhwaError> for WebcamError {
    fn from(e: nokhwa::NokhwaError) -> Self {
        WebcamError::NokhwaError(e)
    }
}

/// Convert a brightness value (0-255) to an ASCII character
fn brightness_to_char(brightness: u8) -> char {
    // Invert so bright = light characters, dark = dense characters
    let inverted = 255 - brightness;
    let index = (inverted as usize * (ASCII_RAMP.len() - 1)) / 255;
    ASCII_RAMP[index]
}

/// Parse a device path like "/dev/video0" to extract the camera index
fn parse_device_index(device: &str) -> Result<u32, WebcamError> {
    // Handle /dev/video0, /dev/video1, etc.
    if let Some(suffix) = device.strip_prefix("/dev/video") {
        suffix.parse::<u32>()
            .map_err(|_| WebcamError::InvalidDevice(device.to_string()))
    } else {
        // Try parsing as a raw number
        device.parse::<u32>()
            .map_err(|_| WebcamError::InvalidDevice(device.to_string()))
    }
}

/// Capture a single frame from the webcam and convert to ASCII art lines
pub fn capture_ascii_snapshot(device: Option<&str>) -> Result<Vec<String>, WebcamError> {
    let device = device.ok_or(WebcamError::NotConfigured)?;
    
    let index = parse_device_index(device)?;
    let camera_index = CameraIndex::Index(index);
    // Request lower resolution (640x480) to reduce CPU usage
    let format = CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 30);
    let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(format));
    
    let mut camera = match Camera::new(camera_index.clone(), requested) {
        Ok(cam) => cam,
        Err(e) => {
            eprintln!("Snapshot: Preferred format failed ({}), trying fallback...", e);
            let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
            Camera::new(camera_index, requested)?
        }
    };
    
    // Start the camera stream
    eprintln!("Taking snapshot: Opening webcam stream...");
    if let Err(e) = camera.open_stream() {
        eprintln!("Snapshot failed: Could not open stream: {}", e);
        return Err(WebcamError::from(e));
    }
    
    // Capture a frame
    let frame = match camera.frame() {
        Ok(f) => f,
        Err(e) => {
             eprintln!("Snapshot failed: Could not capture frame: {}", e);
             let _ = camera.stop_stream();
             return Err(WebcamError::from(e));
        }
    };
    let decoded = frame.decode_image::<RgbFormat>()?;
    
    // Stop the stream
    eprintln!("Snapshot complete: Closing webcam stream...");
    let _ = camera.stop_stream();
    
    // Convert to our ASCII art
    let image = DynamicImage::ImageRgb8(decoded);
    Ok(image_to_ascii(&image, IMAGE_HEIGHT))
}

/// A persistent webcam stream handler
pub struct WebcamStream {
    camera: Camera,
}

impl WebcamStream {
    pub fn new(device: Option<&str>) -> Result<Self, WebcamError> {
        let device = device.ok_or(WebcamError::NotConfigured)?;
        let index = parse_device_index(device)?;
        let camera_index = CameraIndex::Index(index);
        
        // First try: Preference for 640x480 MJPEG @ 30fps
        let format = CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 30);
        let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(format));
        
        let camera = match Camera::new(camera_index.clone(), requested) {
            Ok(cam) => cam,
            Err(e) => {
                eprintln!("Preferred format failed ({}), trying fallback...", e);
                // Fallback: Try with no specific format requirements (let the driver pick)
                let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
                Camera::new(camera_index, requested)?
            }
        };
        
        Ok(Self { camera })
    }

    pub fn start(&mut self) -> Result<(), WebcamError> {
        eprintln!("Starting webcam stream...");
        match self.camera.open_stream() {
            Ok(_) => {
                eprintln!("Webcam stream started successfully.");
                Ok(())
            }
            Err(e) => {
                eprintln!("Failed to start webcam stream: {}", e);
                Err(WebcamError::from(e))
            }
        }
    }

    pub fn stop(&mut self) -> Result<(), WebcamError> {
        if self.camera.is_stream_open() {
            eprintln!("Stopping webcam stream...");
            match self.camera.stop_stream() {
                Ok(_) => {
                    eprintln!("Webcam stream stopped.");
                    Ok(())
                }
                Err(e) => {
                    eprintln!("Failed to stop webcam stream: {}", e);
                    Err(WebcamError::from(e))
                }
            }
        } else {
            Ok(())
        }
    }

    pub fn capture_frame(&mut self) -> Result<Vec<String>, WebcamError> {
        let frame = self.camera.frame()?;
        let decoded = frame.decode_image::<RgbFormat>()?;
        let image = DynamicImage::ImageRgb8(decoded);
        Ok(image_to_ascii(&image, CALL_IMAGE_HEIGHT))
    }
}

/// Apply contrast enhancement to a grayscale image
/// Uses histogram stretching + S-curve for extra punch
fn enhance_contrast(image: &image::GrayImage) -> image::GrayImage {
    // Find min and max pixel values (use percentiles to ignore outliers)
    let mut histogram = [0u32; 256];
    let total_pixels = (image.width() * image.height()) as u32;
    
    for pixel in image.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    
    // Find 2nd and 98th percentile for robust stretching
    let low_threshold = total_pixels / 50;  // 2%
    let high_threshold = total_pixels - (total_pixels / 50);  // 98%
    
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
    
    // Avoid division by zero
    if max_val <= min_val {
        max_val = min_val + 1;
    }
    
    let range = (max_val - min_val) as f32;
    let mut result = image.clone();
    
    for pixel in result.pixels_mut() {
        let val = pixel[0];
        
        // Clamp to percentile range and stretch
        let clamped = val.max(min_val).min(max_val);
        let normalized = (clamped - min_val) as f32 / range;
        
        // Apply S-curve for extra contrast (attempt to differentiate midtones)
        // S-curve: 3x^2 - 2x^3 (smoothstep)
        let curved = normalized * normalized * (3.0 - 2.0 * normalized);
        
        pixel[0] = (curved * 255.0) as u8;
    }
    
    result
}

/// Convert an image to ASCII art lines
fn image_to_ascii(image: &DynamicImage, height_rows: u32) -> Vec<String> {
    // Calculate target dimensions accounting for character aspect ratio (~2:1)
    // We sample 2 vertical pixels for each character row
    let target_width = IMAGE_WIDTH;
    let target_height = height_rows * 2; // Double because we'll sample 2 rows per char
    
    // Resize and crop to fill the target dimensions FIRST
    // This drastically reduces the number of pixels for subsequent processing
    let resized = image.resize_to_fill(target_width, target_height, FilterType::Triangle);
    
    // Convert to grayscale
    let gray = resized.to_luma8();
    
    // Enhance contrast (now fast because image is tiny)
    let enhanced = enhance_contrast(&gray);
    
    // Calculate padding for centering
    let padding = (DISPLAY_WIDTH - IMAGE_WIDTH as usize) / 2;
    let pad_str: String = " ".repeat(padding);
    
    let mut lines = Vec::with_capacity(height_rows as usize);
    
    // Process 2 rows at a time, averaging them for each character row
    for row in 0..height_rows {
        let mut line = String::with_capacity(DISPLAY_WIDTH);
        line.push_str(&pad_str); // Left padding
        
        for col in 0..IMAGE_WIDTH {
            // Average the two vertical pixels for this character position
            let y1 = row * 2;
            let y2 = row * 2 + 1;
            
            let p1 = enhanced.get_pixel(col, y1)[0] as u16;
            let p2 = if y2 < target_height {
                enhanced.get_pixel(col, y2)[0] as u16
            } else {
                p1
            };
            
            let avg = ((p1 + p2) / 2) as u8;
            line.push(brightness_to_char(avg));
        }
        
        lines.push(line);
    }
    
    lines
}

/// List available cameras (for debugging)
#[allow(dead_code)]
pub fn list_cameras() -> Result<Vec<String>, WebcamError> {
    let cameras = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;
    Ok(cameras.iter().map(|c| format!("{}: {}", c.index(), c.human_name())).collect())
}
