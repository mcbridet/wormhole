//! Webcam capture and ASCII art conversion for VT100/VT220/VT340 terminals.

use crate::graphics::{
    DecGraphicsChar, SHIFT_IN, SHIFT_OUT, brightness_to_drcs_char, image_to_sixel,
};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use nokhwa::{
    Camera,
    pixel_format::RgbFormat,
    utils::{
        CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    },
};
use std::thread;
use tokio::sync::{mpsc, oneshot};

/// Rendering mode for webcam output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Pure ASCII characters (VT100 compatible)
    Ascii,
    /// DRCS custom characters for smoother shading (VT220+)
    Drcs,
    /// Sixel bitmap graphics (VT340) with configurable gray levels
    Sixel { shades: u8 },
}

impl RenderMode {
    /// Create a RenderMode from terminal mode string and config
    pub fn from_terminal_mode(mode: &str, sixel_shades: u8) -> Self {
        match mode {
            "vt340" => RenderMode::Sixel {
                shades: sixel_shades.clamp(2, 64),
            },
            "vt220" => RenderMode::Drcs,
            _ => RenderMode::Ascii,
        }
    }
}

/// Raw grayscale frame data for network transmission
/// Contains pre-processed (resized, cropped, contrast-enhanced) grayscale pixels
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

/// ASCII characters ordered by visual density (light to dark)
#[allow(dead_code)]
const ASCII_RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// Convert a brightness value (0-255) to an enhanced character (char, is_dec_graphics)
fn brightness_to_enhanced_char(brightness: u8) -> (char, bool) {
    // Don't invert for light-on-dark terminals
    // Bright (255) -> Dense char (@) -> White pixel
    // Dark (0) -> Space ( ) -> Black pixel

    // Enhanced ramp mixing ASCII and DEC graphics
    // 0: Space (ASCII)
    // 1: Bullet (DEC) - Small dot
    // 2: . (ASCII)
    // 3: : (ASCII)
    // 4: + (ASCII)
    // 5: Checkerboard (DEC) - 50%
    // 6: # (ASCII)
    // 7: @ (ASCII)

    match brightness {
        0..=40 => (' ', false),
        41..=70 => (DecGraphicsChar::Bullet.as_dec_char(), true),
        71..=100 => ('.', false),
        101..=130 => (':', false),
        131..=160 => ('+', false),
        161..=190 => (DecGraphicsChar::Checkerboard.as_dec_char(), true),
        191..=220 => ('#', false),
        _ => ('@', false),
    }
}

/// Height in terminal rows
const IMAGE_HEIGHT: u32 = 16;
/// Height in terminal rows for Call mode
const CALL_IMAGE_HEIGHT: u32 = 22;

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
#[allow(dead_code)]
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
        suffix
            .parse::<u32>()
            .map_err(|_| WebcamError::InvalidDevice(device.to_string()))
    } else {
        // Try parsing as a raw number
        device
            .parse::<u32>()
            .map_err(|_| WebcamError::InvalidDevice(device.to_string()))
    }
}

/// Capture a single frame from the webcam and convert to ASCII art lines
pub fn capture_ascii_snapshot(
    device: Option<&str>,
    render_mode: RenderMode,
    display_width: usize,
) -> Result<Vec<String>, WebcamError> {
    let device = device.ok_or(WebcamError::NotConfigured)?;

    let index = parse_device_index(device)?;
    let camera_index = CameraIndex::Index(index);
    // Request lower resolution (640x480) to reduce CPU usage
    let format = CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 30);
    let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(format));

    let mut camera = match Camera::new(camera_index.clone(), requested) {
        Ok(cam) => cam,
        Err(e) => {
            eprintln!(
                "Snapshot: Preferred format failed ({}), trying fallback...",
                e
            );
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
    Ok(image_to_output(
        &image,
        IMAGE_HEIGHT,
        render_mode,
        display_width,
    ))
}

#[allow(dead_code)]
enum WebcamCommand {
    Start,
    Stop,
    CaptureFrame {
        render_mode: RenderMode,
        width: usize,
        reply: oneshot::Sender<Result<Vec<String>, WebcamError>>,
    },
    CaptureRawFrame {
        width: usize,
        reply: oneshot::Sender<Result<RawFrame, WebcamError>>,
    },
    Snapshot {
        device: String,
        render_mode: RenderMode,
        width: usize,
        reply: oneshot::Sender<Result<Vec<String>, WebcamError>>,
    },
}

/// A persistent webcam stream handler (internal)
struct WebcamDevice {
    camera: Camera,
}

impl WebcamDevice {
    pub fn new(device: Option<&str>) -> Result<Self, WebcamError> {
        let device = device.ok_or(WebcamError::NotConfigured)?;
        let index = parse_device_index(device)?;
        let camera_index = CameraIndex::Index(index);

        // First try: Preference for 1280x720 MJPEG @ 30fps (16:9 aspect ratio)
        let format = CameraFormat::new(Resolution::new(1280, 720), FrameFormat::MJPEG, 30);
        let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(format));

        let camera = match Camera::new(camera_index.clone(), requested) {
            Ok(cam) => cam,
            Err(e) => {
                eprintln!("Preferred format failed ({}), trying fallback...", e);
                // Fallback: Try 640x480
                let format = CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 30);
                let requested =
                    RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(format));
                match Camera::new(camera_index.clone(), requested) {
                    Ok(cam) => cam,
                    Err(_) => {
                        // Final fallback
                        let requested =
                            RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
                        Camera::new(camera_index, requested)?
                    }
                }
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

    pub fn capture_frame(
        &mut self,
        render_mode: RenderMode,
        display_width: usize,
    ) -> Result<Vec<String>, WebcamError> {
        let frame = self.camera.frame()?;
        let decoded = frame.decode_image::<RgbFormat>()?;
        let image = DynamicImage::ImageRgb8(decoded);
        Ok(image_to_output(
            &image,
            CALL_IMAGE_HEIGHT,
            render_mode,
            display_width,
        ))
    }

    /// Capture a frame and return raw grayscale data for network transmission
    pub fn capture_raw_frame(&mut self, display_width: usize) -> Result<RawFrame, WebcamError> {
        let frame = self.camera.frame()?;
        let decoded = frame.decode_image::<RgbFormat>()?;
        let image = DynamicImage::ImageRgb8(decoded);
        Ok(image_to_raw_frame(&image, CALL_IMAGE_HEIGHT, display_width))
    }
}

/// Thread-safe handle to the webcam
pub struct Webcam {
    tx: mpsc::Sender<WebcamCommand>,
}

impl Webcam {
    pub fn new(device: Option<String>) -> Self {
        let (tx, mut rx) = mpsc::channel(32);

        thread::spawn(move || {
            let mut device_instance = if let Some(dev) = &device {
                WebcamDevice::new(Some(dev)).ok()
            } else {
                None
            };

            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    WebcamCommand::Start => {
                        if let Some(dev) = &mut device_instance {
                            let _ = dev.start();
                        }
                    }
                    WebcamCommand::Stop => {
                        if let Some(dev) = &mut device_instance {
                            let _ = dev.stop();
                        }
                    }
                    WebcamCommand::CaptureFrame {
                        render_mode,
                        width,
                        reply,
                    } => {
                        let res = if let Some(dev) = &mut device_instance {
                            dev.capture_frame(render_mode, width)
                        } else {
                            Err(WebcamError::NotConfigured)
                        };
                        let _ = reply.send(res);
                    }
                    WebcamCommand::CaptureRawFrame { width, reply } => {
                        let res = if let Some(dev) = &mut device_instance {
                            dev.capture_raw_frame(width)
                        } else {
                            Err(WebcamError::NotConfigured)
                        };
                        let _ = reply.send(res);
                    }
                    WebcamCommand::Snapshot {
                        device,
                        render_mode,
                        width,
                        reply,
                    } => {
                        // Stop stream if running to release device
                        let mut was_streaming = false;
                        if let Some(dev) = &mut device_instance
                            && dev.camera.is_stream_open()
                        {
                            was_streaming = true;
                            let _ = dev.stop();
                        }

                        let res = capture_ascii_snapshot(Some(&device), render_mode, width);

                        // Restart stream if it was running
                        if was_streaming && let Some(dev) = &mut device_instance {
                            let _ = dev.start();
                        }

                        let _ = reply.send(res);
                    }
                }
            }
        });

        Self { tx }
    }

    pub async fn start(&self) {
        let _ = self.tx.send(WebcamCommand::Start).await;
    }

    pub async fn stop(&self) {
        let _ = self.tx.send(WebcamCommand::Stop).await;
    }

    /// Capture a pre-rendered frame (for local display only)
    #[allow(dead_code)]
    pub async fn capture_frame(
        &self,
        render_mode: RenderMode,
        width: usize,
    ) -> Result<Vec<String>, WebcamError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(WebcamCommand::CaptureFrame {
                render_mode,
                width,
                reply: tx,
            })
            .await
            .map_err(|_| WebcamError::NotConfigured)?;
        rx.await.map_err(|_| WebcamError::NotConfigured)?
    }

    /// Capture a raw grayscale frame for network transmission
    pub async fn capture_raw_frame(&self, width: usize) -> Result<RawFrame, WebcamError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(WebcamCommand::CaptureRawFrame { width, reply: tx })
            .await
            .map_err(|_| WebcamError::NotConfigured)?;
        rx.await.map_err(|_| WebcamError::NotConfigured)?
    }

    pub async fn take_snapshot(
        &self,
        device: String,
        render_mode: RenderMode,
        width: usize,
    ) -> Result<Vec<String>, WebcamError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(WebcamCommand::Snapshot {
                device,
                render_mode,
                width,
                reply: tx,
            })
            .await
            .map_err(|_| WebcamError::NotConfigured)?;
        rx.await.map_err(|_| WebcamError::NotConfigured)?
    }
}

/// Apply contrast enhancement to a grayscale image
/// Uses histogram stretching + S-curve for extra punch
fn enhance_contrast(image: &image::GrayImage) -> image::GrayImage {
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

    // Prevent over-stretching of dark images (noise amplification)
    // If the dynamic range is small and mostly dark, don't stretch it to full white
    if max_val < 100 {
        max_val = 100;
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

/// Prepare common image dimensions for rendering
/// Returns (target_width, target_height) for the processed image
fn calculate_frame_dimensions(
    image: &DynamicImage,
    height_rows: u32,
    display_width: usize,
) -> (u32, u32) {
    // Calculate target dimensions accounting for character aspect ratio (~2:1)
    // We sample 2 vertical pixels for each character row
    let target_height = height_rows * 2;

    let (img_w, img_h) = image.dimensions();
    let mut aspect = img_w as f32 / img_h as f32;

    // If we have a wide display (132 cols), let's try to be at least 16:9
    if display_width > 100 {
        aspect = aspect.max(1.77);
    }

    // Calculate ideal width to maintain aspect ratio
    let ideal_width = (target_height as f32 * aspect) as u32;

    // Constrain to display width (minus padding)
    let max_width = (display_width.saturating_sub(4)) as u32;
    let target_width = ideal_width.min(max_width);

    (target_width, target_height)
}

/// Process an image to raw grayscale frame data for network transmission
/// The image is resized, cropped, converted to grayscale, and contrast-enhanced
fn image_to_raw_frame(image: &DynamicImage, height_rows: u32, display_width: usize) -> RawFrame {
    let (target_width, target_height) =
        calculate_frame_dimensions(image, height_rows, display_width);

    // Resize and crop to fill the target dimensions
    let resized = image.resize_to_fill(target_width, target_height, FilterType::Triangle);

    // Convert to grayscale
    let gray = resized.to_luma8();

    // Enhance contrast
    let enhanced = enhance_contrast(&gray);

    RawFrame {
        width: target_width as u16,
        height: target_height as u16,
        pixels: enhanced.into_raw(),
    }
}

/// Render a raw grayscale frame to terminal output lines
/// This allows the receiver to render according to their terminal capabilities
pub fn raw_frame_to_output(
    frame: &RawFrame,
    render_mode: RenderMode,
    sixel_shades: u8,
) -> Vec<String> {
    let width = frame.width as u32;
    let height = frame.height as u32;
    let height_rows = height / 2; // Each character row represents 2 pixel rows

    // For sixel mode, reconstruct a DynamicImage and use sixel encoder
    if let RenderMode::Sixel { shades: _ } = render_mode {
        use crate::graphics::SixelConfig;
        use image::GrayImage;

        if let Some(gray_image) = GrayImage::from_raw(width, height, frame.pixels.clone()) {
            let image = DynamicImage::ImageLuma8(gray_image);
            let config = SixelConfig {
                gray_levels: sixel_shades,
                ..Default::default()
            };
            let sixel_output =
                image_to_sixel(&image, height_rows, width as usize + 4, Some(&config));
            return vec![sixel_output];
        }
        // Fallback if reconstruction fails
        return vec!["[sixel render error]".to_string()];
    }

    // For ASCII/DRCS modes, render from raw pixel data
    let use_drcs = render_mode == RenderMode::Drcs;
    let mut lines = Vec::with_capacity(height_rows as usize);

    // Process 2 rows at a time, averaging them for each character row
    for row in 0..height_rows {
        let mut line = String::with_capacity(width as usize + 10);

        if use_drcs {
            line.push_str(SHIFT_OUT);

            for col in 0..width {
                let y1 = row * 2;
                let y2 = row * 2 + 1;

                let idx1 = (y1 * width + col) as usize;
                let idx2 = (y2 * width + col) as usize;

                let p1 = frame.pixels.get(idx1).copied().unwrap_or(0) as u16;
                let p2 = if y2 < height {
                    frame.pixels.get(idx2).copied().unwrap_or(0) as u16
                } else {
                    p1
                };

                let avg = ((p1 + p2) / 2) as u8;
                let char = brightness_to_drcs_char(avg);
                line.push(char);
            }

            line.push_str(SHIFT_IN);
        } else {
            // Enhanced ASCII mode
            let mut current_is_dec = false;

            for col in 0..width {
                let y1 = row * 2;
                let y2 = row * 2 + 1;

                let idx1 = (y1 * width + col) as usize;
                let idx2 = (y2 * width + col) as usize;

                let p1 = frame.pixels.get(idx1).copied().unwrap_or(0) as u16;
                let p2 = if y2 < height {
                    frame.pixels.get(idx2).copied().unwrap_or(0) as u16
                } else {
                    p1
                };

                let avg = ((p1 + p2) / 2) as u8;
                let (char, is_dec) = brightness_to_enhanced_char(avg);

                if is_dec != current_is_dec {
                    if is_dec {
                        line.push_str(SHIFT_OUT);
                    } else {
                        line.push_str(SHIFT_IN);
                    }
                    current_is_dec = is_dec;
                }
                line.push(char);
            }

            if current_is_dec {
                line.push_str(SHIFT_IN);
            }
        }

        lines.push(line);
    }

    lines
}

/// Convert an image to terminal output lines (ASCII, DRCS, or Sixel)
fn image_to_output(
    image: &DynamicImage,
    height_rows: u32,
    render_mode: RenderMode,
    display_width: usize,
) -> Vec<String> {
    // For sixel mode, we render directly to sixel format
    if let RenderMode::Sixel { shades } = render_mode {
        use crate::graphics::SixelConfig;
        let config = SixelConfig {
            gray_levels: shades,
            ..Default::default()
        };
        let sixel_output = image_to_sixel(image, height_rows, display_width, Some(&config));
        // Return as a single "line" - sixel handles its own positioning
        return vec![sixel_output];
    }

    // For ASCII/DRCS modes, use character-based rendering
    let use_drcs = render_mode == RenderMode::Drcs;

    // Calculate target dimensions accounting for character aspect ratio (~2:1)
    // We sample 2 vertical pixels for each character row
    let target_height = height_rows * 2; // Double because we'll sample 2 rows per char

    // Calculate target width based on image aspect ratio
    // 1 char width = 1 unit, 1 char height = 2 units (roughly)
    // So target_height pixels represents height_rows characters.
    // To maintain aspect ratio:
    // width_chars = height_chars * (image_w / image_h) * (char_h / char_w)
    // width_chars = height_rows * (image_w / image_h) * 2.0
    // Or simply: target_width_pixels = target_height_pixels * (image_w / image_h)
    // Since we map 1 pixel to 1 char horizontally, and 2 pixels to 1 char vertically.

    let (img_w, img_h) = image.dimensions();
    let mut aspect = img_w as f32 / img_h as f32;

    // If we have a wide display (132 cols), let's try to be at least 16:9
    if display_width > 100 {
        aspect = aspect.max(1.77);
    }

    // Calculate ideal width to maintain aspect ratio
    let ideal_width = (target_height as f32 * aspect) as u32;

    // Constrain to display width (minus padding)
    let max_width = (display_width.saturating_sub(4)) as u32;
    let target_width = ideal_width.min(max_width);

    // Resize and crop to fill the target dimensions FIRST
    // This drastically reduces the number of pixels for subsequent processing
    let resized = image.resize_to_fill(target_width, target_height, FilterType::Triangle);

    // Convert to grayscale
    let gray = resized.to_luma8();

    // Enhance contrast (now fast because image is tiny)
    let enhanced = enhance_contrast(&gray);

    let mut lines = Vec::with_capacity(height_rows as usize);

    // Process 2 rows at a time, averaging them for each character row
    for row in 0..height_rows {
        let mut line = String::with_capacity(target_width as usize + 10);

        if use_drcs {
            line.push_str(SHIFT_OUT);

            for col in 0..target_width {
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
                let char = brightness_to_drcs_char(avg);
                line.push(char);
            }

            line.push_str(SHIFT_IN);
        } else {
            // Enhanced ASCII mode (mix of ASCII and DEC graphics)
            let mut current_is_dec = false;

            for col in 0..target_width {
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
                let (char, is_dec) = brightness_to_enhanced_char(avg);

                if is_dec != current_is_dec {
                    if is_dec {
                        line.push_str(SHIFT_OUT);
                    } else {
                        line.push_str(SHIFT_IN);
                    }
                    current_is_dec = is_dec;
                }
                line.push(char);
            }

            if current_is_dec {
                line.push_str(SHIFT_IN);
            }
        }

        lines.push(line);
    }

    lines
}

/// List available cameras (for debugging)
#[allow(dead_code)]
pub fn list_cameras() -> Result<Vec<String>, WebcamError> {
    let cameras = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;
    Ok(cameras
        .iter()
        .map(|c| format!("{}: {}", c.index(), c.human_name()))
        .collect())
}
