//! Session logging functionality for chat and AI tabs.

use chrono::{Local, NaiveDate};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// Logger that writes to date-stamped log files per tab.
pub struct SessionLogger {
    log_dir: PathBuf,
    chat_file: Option<File>,
    ai_file: Option<File>,
    chat_date: Option<NaiveDate>,
    ai_date: Option<NaiveDate>,
}

impl SessionLogger {
    /// Create a new session logger with the specified log directory.
    /// Returns None if log_dir is None (logging disabled).
    /// Supports both absolute and relative paths (relative to current working directory).
    pub fn new(log_dir: Option<&str>) -> Option<Self> {
        let log_dir = log_dir?;
        let log_path = PathBuf::from(log_dir);

        // Resolve relative paths to absolute
        let log_path = if log_path.is_absolute() {
            log_path
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&log_path))
                .unwrap_or(log_path)
        };

        // Create log directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(&log_path) {
            eprintln!(
                "Warning: Failed to create log directory '{}': {}",
                log_path.display(),
                e
            );
            return None;
        }

        Some(Self {
            log_dir: log_path,
            chat_file: None,
            ai_file: None,
            chat_date: None,
            ai_date: None,
        })
    }

    /// Get the log file path for a tab and date.
    fn log_file_path(log_dir: &std::path::Path, tab: &str, date: NaiveDate) -> PathBuf {
        let filename = format!("{}-{}.log", tab, date.format("%Y%m%d"));
        log_dir.join(filename)
    }

    /// Ensure the log file for a tab is open and matches the current date.
    /// Returns a mutable reference to the file if successful.
    fn ensure_file(&mut self, tab: &str) -> Option<&mut File> {
        let today = Local::now().date_naive();

        let (file_opt, date_opt) = match tab {
            "chat" => (&mut self.chat_file, &mut self.chat_date),
            "ai" => (&mut self.ai_file, &mut self.ai_date),
            _ => return None,
        };

        // Check if we need to open a new file (no file, or date changed)
        let needs_new_file = match *date_opt {
            None => true,
            Some(d) => d != today,
        };

        if needs_new_file {
            // Close existing file
            *file_opt = None;
            *date_opt = None;

            let path = Self::log_file_path(&self.log_dir, tab, today);
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => {
                    *file_opt = Some(file);
                    *date_opt = Some(today);
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to open log file '{}': {}",
                        path.display(),
                        e
                    );
                    return None;
                }
            }
        }

        file_opt.as_mut()
    }

    /// Log a message to the chat tab log.
    pub fn log_chat(&mut self, message: &str) {
        if let Some(file) = self.ensure_file("chat") {
            let _ = writeln!(file, "{}", message);
        }
    }

    /// Log a message to the AI tab log.
    pub fn log_ai(&mut self, message: &str) {
        if let Some(file) = self.ensure_file("ai") {
            let _ = writeln!(file, "{}", message);
        }
    }
}
