//! Tunes tab: browse and play audio files from a directory.

use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

use crate::terminal::esc;

/// Tunes display area bounds (full box, rows 2-23)
const TUNES_REGION_START: usize = 2;
const TUNES_REGION_END: usize = 23;

/// Visible lines for file listing (minus 1 for status line at bottom)
const TUNES_VISIBLE_LINES: usize = TUNES_REGION_END - TUNES_REGION_START;

/// Supported audio file extensions
const SUPPORTED_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "ogg"];

/// Playback state shared between threads
#[derive(Debug, Clone)]
pub enum PlaybackState {
    Stopped,
    Playing(String), // Currently playing filename
    Paused(String),  // Paused filename
}

/// Playback timing info
#[derive(Debug, Clone)]
struct PlaybackTiming {
    /// Total duration of the track
    total_duration: Option<Duration>,
    /// When playback started
    start_time: Instant,
    /// Accumulated pause time
    paused_duration: Duration,
    /// When pause started (if paused)
    pause_start: Option<Instant>,
}

impl PlaybackTiming {
    fn new(total_duration: Option<Duration>) -> Self {
        Self {
            total_duration,
            start_time: Instant::now(),
            paused_duration: Duration::ZERO,
            pause_start: None,
        }
    }

    fn elapsed(&self) -> Duration {
        let raw_elapsed = self.start_time.elapsed();
        let current_pause = self
            .pause_start
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);
        raw_elapsed.saturating_sub(self.paused_duration + current_pause)
    }

    fn remaining(&self) -> Option<Duration> {
        self.total_duration
            .map(|total| total.saturating_sub(self.elapsed()))
    }

    fn pause(&mut self) {
        if self.pause_start.is_none() {
            self.pause_start = Some(Instant::now());
        }
    }

    fn resume(&mut self) {
        if let Some(pause_start) = self.pause_start.take() {
            self.paused_duration += pause_start.elapsed();
        }
    }
}

/// Audio player that runs in a background thread
pub struct AudioPlayer {
    /// Shared playback state
    state: Arc<Mutex<PlaybackState>>,
    /// Sink for controlling playback (wrapped in Arc<Mutex> for thread safety)
    sink: Arc<Mutex<Option<Sink>>>,
    /// Playback timing info
    timing: Arc<Mutex<Option<PlaybackTiming>>>,
    /// Keep the stream alive (must not be dropped while playing)
    _stream: OutputStream,
    /// Stream handle for creating sinks
    stream_handle: OutputStreamHandle,
}

impl AudioPlayer {
    /// Create a new audio player
    pub fn new() -> Result<Self, String> {
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to open audio output: {}", e))?;

        Ok(Self {
            state: Arc::new(Mutex::new(PlaybackState::Stopped)),
            sink: Arc::new(Mutex::new(None)),
            timing: Arc::new(Mutex::new(None)),
            _stream: stream,
            stream_handle,
        })
    }

    /// Play an audio file
    pub fn play(&self, path: &Path) -> Result<(), String> {
        // Stop any current playback
        self.stop();

        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let reader = BufReader::new(file);

        let source = Decoder::new(reader).map_err(|e| format!("Failed to decode audio: {}", e))?;

        // Get total duration before consuming the source
        let total_duration = source.total_duration();

        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| format!("Failed to create audio sink: {}", e))?;

        sink.append(source);

        // Store filename for display
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        // Update state
        {
            let mut state = self.state.lock().unwrap();
            *state = PlaybackState::Playing(filename);
        }

        // Update timing
        {
            let mut timing = self.timing.lock().unwrap();
            *timing = Some(PlaybackTiming::new(total_duration));
        }

        // Store sink for control
        {
            let mut sink_guard = self.sink.lock().unwrap();
            *sink_guard = Some(sink);
        }

        // Start a thread to monitor playback completion
        let state_clone = Arc::clone(&self.state);
        let sink_clone = Arc::clone(&self.sink);
        let timing_clone = Arc::clone(&self.timing);
        thread::spawn(move || {
            loop {
                thread::sleep(std::time::Duration::from_millis(100));

                let sink_guard = sink_clone.lock().unwrap();
                if let Some(ref sink) = *sink_guard {
                    if sink.empty() {
                        drop(sink_guard);
                        let mut state = state_clone.lock().unwrap();
                        *state = PlaybackState::Stopped;
                        let mut timing = timing_clone.lock().unwrap();
                        *timing = None;
                        let mut sink_guard = sink_clone.lock().unwrap();
                        *sink_guard = None;
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        Ok(())
    }

    /// Stop playback
    pub fn stop(&self) {
        let mut sink_guard = self.sink.lock().unwrap();
        if let Some(sink) = sink_guard.take() {
            sink.stop();
        }

        let mut state = self.state.lock().unwrap();
        *state = PlaybackState::Stopped;

        let mut timing = self.timing.lock().unwrap();
        *timing = None;
    }

    /// Toggle pause/resume
    pub fn toggle_pause(&self) {
        let sink_guard = self.sink.lock().unwrap();
        if let Some(ref sink) = *sink_guard {
            if sink.is_paused() {
                sink.play();
                let mut state = self.state.lock().unwrap();
                if let PlaybackState::Paused(filename) = state.clone() {
                    *state = PlaybackState::Playing(filename);
                }
                // Resume timing
                let mut timing = self.timing.lock().unwrap();
                if let Some(ref mut t) = *timing {
                    t.resume();
                }
            } else {
                sink.pause();
                let mut state = self.state.lock().unwrap();
                if let PlaybackState::Playing(filename) = state.clone() {
                    *state = PlaybackState::Paused(filename);
                }
                // Pause timing
                let mut timing = self.timing.lock().unwrap();
                if let Some(ref mut t) = *timing {
                    t.pause();
                }
            }
        }
    }

    /// Get current playback state
    pub fn state(&self) -> PlaybackState {
        self.state.lock().unwrap().clone()
    }

    /// Get remaining playback time
    pub fn remaining_time(&self) -> Option<Duration> {
        let timing = self.timing.lock().unwrap();
        timing.as_ref().and_then(|t| t.remaining())
    }

    /// Check if currently playing
    #[allow(dead_code)]
    pub fn is_playing(&self) -> bool {
        matches!(self.state(), PlaybackState::Playing(_))
    }

    /// Check if paused
    #[allow(dead_code)]
    pub fn is_paused(&self) -> bool {
        matches!(self.state(), PlaybackState::Paused(_))
    }
}

/// State for the Tunes tab
pub struct TunesState {
    /// Directory containing tune files
    directory: PathBuf,
    /// List of files in the directory
    files: Vec<String>,
    /// Currently selected index
    selected: usize,
    /// Scroll offset for display
    scroll_offset: usize,
    /// Terminal width
    width: usize,
    /// Audio player
    player: Option<AudioPlayer>,
}

impl TunesState {
    /// Create a new TunesState from a directory path
    pub fn new(directory: &str, width: usize) -> Self {
        let directory = PathBuf::from(directory);
        let files = Self::scan_directory(&directory);

        // Try to create audio player
        let player = match AudioPlayer::new() {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("Warning: Failed to initialize audio player: {}", e);
                None
            }
        };

        Self {
            directory,
            files,
            selected: 0,
            scroll_offset: 0,
            width,
            player,
        }
    }

    /// Check if a directory is configured, exists, and has supported audio files
    pub fn is_available(directory: Option<&str>) -> bool {
        match directory {
            Some(dir) => {
                let path = Path::new(dir);
                if !path.is_dir() {
                    return false;
                }
                // Check if directory has any supported audio files
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let entry_path = entry.path();
                        if entry_path.is_file() && Self::is_supported_audio_file(&entry_path) {
                            return true;
                        }
                    }
                }
                false
            }
            None => false,
        }
    }

    /// Check if a file has a supported audio extension
    fn is_supported_audio_file(path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Skip hidden files
            if name.starts_with('.') {
                return false;
            }
        }

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
        } else {
            false
        }
    }

    /// Scan directory for supported audio files (non-recursive)
    fn scan_directory(directory: &Path) -> Vec<String> {
        let mut files = Vec::new();

        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && Self::is_supported_audio_file(&path)
                    && let Some(name) = path.file_name()
                    && let Some(name_str) = name.to_str()
                {
                    files.push(name_str.to_string());
                }
            }
        }

        // Sort alphabetically (case-insensitive)
        files.sort_by_key(|a| a.to_lowercase());
        files
    }

    /// Refresh the file list from the directory
    #[allow(dead_code)]
    pub fn refresh(&mut self) {
        self.files = Self::scan_directory(&self.directory);

        // Ensure selection is still valid
        if self.files.is_empty() {
            self.selected = 0;
            self.scroll_offset = 0;
        } else if self.selected >= self.files.len() {
            self.selected = self.files.len() - 1;
        }

        self.ensure_visible();
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        if !self.files.is_empty() && self.selected < self.files.len() - 1 {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Page up
    pub fn page_up(&mut self) {
        if self.selected >= TUNES_VISIBLE_LINES {
            self.selected -= TUNES_VISIBLE_LINES;
        } else {
            self.selected = 0;
        }
        self.ensure_visible();
    }

    /// Page down
    pub fn page_down(&mut self) {
        if !self.files.is_empty() {
            let new_pos = self.selected + TUNES_VISIBLE_LINES;
            if new_pos < self.files.len() {
                self.selected = new_pos;
            } else {
                self.selected = self.files.len() - 1;
            }
            self.ensure_visible();
        }
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + TUNES_VISIBLE_LINES {
            self.scroll_offset = self.selected - TUNES_VISIBLE_LINES + 1;
        }
    }

    /// Get the currently selected filename
    #[allow(dead_code)]
    pub fn selected_file(&self) -> Option<&str> {
        self.files.get(self.selected).map(|s| s.as_str())
    }

    /// Get the full path to the selected file
    pub fn selected_path(&self) -> Option<PathBuf> {
        self.files
            .get(self.selected)
            .map(|name| self.directory.join(name))
    }

    /// Get the number of files
    #[allow(dead_code)]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Play the currently selected file
    pub fn play_selected(&self) -> Result<(), String> {
        if let Some(ref player) = self.player {
            if let Some(path) = self.selected_path() {
                player.play(&path)
            } else {
                Err("No file selected".to_string())
            }
        } else {
            Err("Audio player not available".to_string())
        }
    }

    /// Stop playback
    pub fn stop(&self) {
        if let Some(ref player) = self.player {
            player.stop();
        }
    }

    /// Toggle pause/resume
    pub fn toggle_pause(&self) {
        if let Some(ref player) = self.player {
            player.toggle_pause();
        }
    }

    /// Get current playback state
    pub fn playback_state(&self) -> PlaybackState {
        if let Some(ref player) = self.player {
            player.state()
        } else {
            PlaybackState::Stopped
        }
    }

    /// Check if currently playing or paused (i.e., has active audio)
    pub fn is_active(&self) -> bool {
        !matches!(self.playback_state(), PlaybackState::Stopped)
    }

    /// Get remaining playback time
    pub fn remaining_time(&self) -> Option<Duration> {
        if let Some(ref player) = self.player {
            player.remaining_time()
        } else {
            None
        }
    }

    /// Format duration as MM:SS
    fn format_duration(d: Duration) -> String {
        let total_secs = d.as_secs();
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    /// Render the tunes list to terminal output
    pub fn render(&self) -> String {
        let mut output = String::new();
        // Content area: column 2 to column (width-1), leaving column 1 and width for borders
        let content_width = self.width - 2;

        // Get current playback state for display
        let playback_state = self.playback_state();
        let playing_file = match &playback_state {
            PlaybackState::Playing(f) => Some(f.as_str()),
            PlaybackState::Paused(f) => Some(f.as_str()),
            PlaybackState::Stopped => None,
        };

        // Clear and render each visible line (leave last line for status)
        for i in 0..TUNES_VISIBLE_LINES {
            let row = TUNES_REGION_START + i;
            output.push_str(&esc::cursor_to(row, 2));

            let file_idx = self.scroll_offset + i;
            if file_idx < self.files.len() {
                let file = &self.files[file_idx];
                let is_selected = file_idx == self.selected;
                let is_playing = playing_file == Some(file.as_str());

                // Build display prefix (playing indicator)
                let prefix = if is_playing {
                    match &playback_state {
                        PlaybackState::Playing(_) => "> ",
                        PlaybackState::Paused(_) => "| ",
                        _ => "  ",
                    }
                } else {
                    "  "
                };

                // Truncate filename if too long
                // content_width is total available, minus 2 for prefix
                let max_name_len = content_width.saturating_sub(2);
                let display_name: String = if file.chars().count() > max_name_len {
                    let truncated: String =
                        file.chars().take(max_name_len.saturating_sub(3)).collect();
                    format!("{}...", truncated)
                } else {
                    file.clone()
                };

                let line_content = format!("{}{}", prefix, display_name);

                if is_selected {
                    // Highlight selected item
                    output.push_str(esc::REVERSE);
                    output.push_str(&line_content);
                    // Pad to fill the line while highlighted
                    let padlen = content_width.saturating_sub(line_content.chars().count());
                    for _ in 0..padlen {
                        output.push(' ');
                    }
                    output.push_str(esc::RESET_ATTRS);
                } else {
                    output.push_str(&line_content);
                    // Clear rest of line
                    let padlen = content_width.saturating_sub(line_content.chars().count());
                    for _ in 0..padlen {
                        output.push(' ');
                    }
                }
            } else {
                // Empty line
                for _ in 0..content_width {
                    output.push(' ');
                }
            }
        }

        // Show status line at bottom
        let status = if self.files.is_empty() {
            "(No audio files found)".to_string()
        } else {
            let nav_hint = format!(" {}/{}", self.selected + 1, self.files.len());
            match &playback_state {
                PlaybackState::Playing(_) => {
                    let time_str = self
                        .remaining_time()
                        .map(Self::format_duration)
                        .unwrap_or_else(|| "--:--".to_string());
                    format!("{} | Playing {} | Pause <Space>", nav_hint, time_str)
                }
                PlaybackState::Paused(_) => {
                    let time_str = self
                        .remaining_time()
                        .map(Self::format_duration)
                        .unwrap_or_else(|| "--:--".to_string());
                    format!("{} | Paused {} | Resume <Space>", nav_hint, time_str)
                }
                PlaybackState::Stopped => {
                    format!("{} | Play <Enter>", nav_hint)
                }
            }
        };

        output.push_str(&esc::cursor_to(TUNES_REGION_END, 2));
        let status_display: String = if status.chars().count() > content_width {
            status.chars().take(content_width).collect()
        } else {
            status
        };
        // Dim the status line
        output.push_str("\x1b[2m"); // Dim attribute
        output.push_str(&status_display);
        // Pad rest of line
        let padlen = content_width.saturating_sub(status_display.chars().count());
        for _ in 0..padlen {
            output.push(' ');
        }
        output.push_str(esc::RESET_ATTRS);

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn test_supported_audio_extensions() {
        assert!(TunesState::is_supported_audio_file(Path::new("song.mp3")));
        assert!(TunesState::is_supported_audio_file(Path::new("song.MP3")));
        assert!(TunesState::is_supported_audio_file(Path::new("song.wav")));
        assert!(TunesState::is_supported_audio_file(Path::new("song.flac")));
        assert!(TunesState::is_supported_audio_file(Path::new("song.ogg")));
        assert!(!TunesState::is_supported_audio_file(Path::new("song.txt")));
        assert!(!TunesState::is_supported_audio_file(Path::new("song.exe")));
        assert!(!TunesState::is_supported_audio_file(Path::new(
            "noextension"
        )));
    }

    #[test]
    fn test_hidden_files_excluded() {
        assert!(!TunesState::is_supported_audio_file(Path::new(
            ".hidden.mp3"
        )));
        assert!(!TunesState::is_supported_audio_file(Path::new(
            ".DS_Store.mp3"
        )));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(TunesState::format_duration(Duration::from_secs(0)), "00:00");
        assert_eq!(
            TunesState::format_duration(Duration::from_secs(30)),
            "00:30"
        );
        assert_eq!(
            TunesState::format_duration(Duration::from_secs(60)),
            "01:00"
        );
        assert_eq!(
            TunesState::format_duration(Duration::from_secs(90)),
            "01:30"
        );
        assert_eq!(
            TunesState::format_duration(Duration::from_secs(3661)),
            "61:01"
        );
    }

    #[test]
    fn test_scan_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let files = TunesState::scan_directory(temp_dir.path());
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_directory_with_audio_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create some audio files (empty, but with correct extensions)
        File::create(temp_dir.path().join("song1.mp3")).unwrap();
        File::create(temp_dir.path().join("song2.wav")).unwrap();
        File::create(temp_dir.path().join("readme.txt")).unwrap(); // Should be excluded

        let files = TunesState::scan_directory(temp_dir.path());
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"song1.mp3".to_string()));
        assert!(files.contains(&"song2.wav".to_string()));
        assert!(!files.contains(&"readme.txt".to_string()));
    }

    #[test]
    fn test_scan_directory_sorts_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();

        File::create(temp_dir.path().join("Zebra.mp3")).unwrap();
        File::create(temp_dir.path().join("apple.mp3")).unwrap();
        File::create(temp_dir.path().join("Banana.mp3")).unwrap();

        let files = TunesState::scan_directory(temp_dir.path());
        assert_eq!(files, vec!["apple.mp3", "Banana.mp3", "Zebra.mp3"]);
    }

    #[test]
    fn test_is_available_nonexistent() {
        assert!(!TunesState::is_available(Some("/nonexistent/path")));
        assert!(!TunesState::is_available(None));
    }

    #[test]
    fn test_is_available_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        assert!(!TunesState::is_available(Some(
            temp_dir.path().to_str().unwrap()
        )));
    }

    #[test]
    fn test_is_available_with_audio() {
        let temp_dir = TempDir::new().unwrap();
        File::create(temp_dir.path().join("song.mp3")).unwrap();
        assert!(TunesState::is_available(Some(
            temp_dir.path().to_str().unwrap()
        )));
    }

    #[test]
    fn test_playback_timing_elapsed() {
        let timing = PlaybackTiming::new(Some(Duration::from_secs(120)));
        // Elapsed should be very small (just created)
        assert!(timing.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn test_playback_timing_remaining() {
        let timing = PlaybackTiming::new(Some(Duration::from_secs(120)));
        let remaining = timing.remaining().unwrap();
        // Remaining should be close to 120 seconds
        assert!(remaining > Duration::from_secs(119));
        assert!(remaining <= Duration::from_secs(120));
    }

    #[test]
    fn test_playback_timing_pause_resume() {
        let mut timing = PlaybackTiming::new(Some(Duration::from_secs(120)));

        // Pause
        timing.pause();
        assert!(timing.pause_start.is_some());

        // Resume
        timing.resume();
        assert!(timing.pause_start.is_none());
        // Paused duration should be very small
        assert!(timing.paused_duration < Duration::from_millis(100));
    }
}
