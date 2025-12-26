use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub serial: SerialConfig,
    pub network: NetworkConfig,
    #[serde(default)]
    pub webcam: WebcamConfig,
    #[serde(default)]
    pub gemini: GeminiConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub logging: LogConfig,
    #[serde(default)]
    pub tunes: TunesConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminalConfig {
    /// Terminal emulation mode: "vt100", "vt220", or "vt340"
    pub mode: String,

    /// Enable 132 column mode (false if unset)
    #[serde(rename = "132_cols", default, deserialize_with = "deserialize_bool")]
    pub cols_132: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            mode: "vt100".to_string(),
            cols_132: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogConfig {
    /// Directory to write log files to (optional, logging disabled if not set)
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TunesConfig {
    /// Directory containing audio/tune files to list
    /// If not set, Tunes tab is disabled
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SerialConfig {
    /// Path to the serial port device (e.g., /dev/ttyUSB0)
    pub port: String,

    /// Baud rate for serial communication
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WebcamConfig {
    /// Path to the webcam device (e.g., /dev/video0)
    /// If not set, webcam feature is disabled
    #[serde(default)]
    pub device: Option<String>,

    /// Target FPS for video streaming
    #[serde(default = "default_fps")]
    pub fps: u32,

    /// Number of grayscale shades for sixel rendering (VT340 mode)
    /// Range: 2-64, default: 8
    #[serde(default = "default_sixel_shades")]
    pub sixel_shades: u8,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GeminiConfig {
    /// Google Gemini API key
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model to use (e.g., "gemini-2.5-flash", "gemini-2.5-pro")
    #[serde(default = "default_gemini_model")]
    pub model: String,

    /// System prompt for the AI assistant
    #[serde(default)]
    pub system_prompt: Option<String>,
}

fn default_gemini_model() -> String {
    "gemini-2.5-flash".to_string()
}

#[derive(Debug, Deserialize)]
pub struct NetworkConfig {
    /// Display name for this node (required)
    pub name: String,

    /// UDP port for P2P communication
    #[serde(default = "default_port")]
    pub port: u16,

    /// Local IP address to bind to (optional, auto-detected if not set)
    #[serde(default)]
    pub bind_ip: Option<String>,

    /// Enable UPnP port forwarding
    #[serde(default = "default_true", deserialize_with = "deserialize_bool")]
    pub upnp: bool,

    /// Peer addresses to connect to on startup (comma-separated)
    #[serde(default)]
    pub peers: String,
}

/// Deserialize a boolean from string (for INI file compatibility)
fn deserialize_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let s = String::deserialize(deserializer)?;
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(D::Error::custom(format!(
            "invalid boolean value '{}', expected true/false/yes/no/on/off/1/0",
            s
        ))),
    }
}

fn default_baud_rate() -> u32 {
    9600
}

fn default_port() -> u16 {
    7890
}

fn default_true() -> bool {
    true
}

fn default_fps() -> u32 {
    5
}

fn default_sixel_shades() -> u8 {
    8
}

impl Config {
    /// Load configuration from an INI file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path.as_ref()).map_err(|e| ConfigError::Io {
            path: path.as_ref().to_path_buf(),
            source: e,
        })?;

        let mut config: Self = serde_ini::from_str(&contents).map_err(|e| ConfigError::Parse {
            path: path.as_ref().to_path_buf(),
            source: e,
        })?;

        // Truncate network name if it's longer than 16 characters
        if config.network.name.chars().count() > 16 {
            config.network.name = config.network.name.chars().take(16).collect();
        }

        // Validate terminal mode
        if config.terminal.mode != "vt100"
            && config.terminal.mode != "vt220"
            && config.terminal.mode != "vt340"
        {
            return Err(ConfigError::InvalidMode(config.terminal.mode));
        }

        // Validate 132 columns mode (only allowed for vt220+)
        if config.terminal.mode == "vt100" && config.terminal.cols_132 {
            return Err(ConfigError::InvalidColumnsConfig);
        }

        Ok(config)
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        source: serde_ini::de::Error,
    },
    InvalidMode(String),
    InvalidColumnsConfig,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io { path, source } => {
                write!(
                    f,
                    "failed to read config file '{}': {}",
                    path.display(),
                    source
                )
            }
            ConfigError::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse config file '{}': {}",
                    path.display(),
                    source
                )
            }
            ConfigError::InvalidMode(mode) => {
                write!(
                    f,
                    "invalid terminal mode '{}', expected vt100, vt220, or vt340",
                    mode
                )
            }
            ConfigError::InvalidColumnsConfig => {
                write!(f, "132 column mode is only supported in vt220+ modes")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io { source, .. } => Some(source),
            ConfigError::Parse { source, .. } => Some(source),
            ConfigError::InvalidMode(_) => None,
            ConfigError::InvalidColumnsConfig => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_config(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn test_load_minimal_config() {
        let config_content = r#"
[serial]
port = /dev/ttyUSB0
baud = 9600

[network]
name = TestUser
port = 9999
"#;
        let file = create_temp_config(config_content);
        let config = Config::load(file.path()).unwrap();

        assert_eq!(config.serial.port, "/dev/ttyUSB0");
        assert_eq!(config.serial.baud_rate, 9600);
        assert_eq!(config.network.name, "TestUser");
        assert_eq!(config.network.port, 9999);
        // Defaults
        assert_eq!(config.terminal.mode, "vt100");
        assert!(!config.terminal.cols_132);
    }

    #[test]
    fn test_load_vt220_config() {
        let config_content = r#"
[serial]
port = /dev/ttyUSB0
baud = 38400

[network]
name = TestUser
port = 9999

[terminal]
mode = vt220
132_cols = true
"#;
        let file = create_temp_config(config_content);
        let config = Config::load(file.path()).unwrap();

        assert_eq!(config.terminal.mode, "vt220");
        assert!(config.terminal.cols_132);
    }

    #[test]
    fn test_invalid_terminal_mode() {
        let config_content = r#"
[serial]
port = /dev/ttyUSB0
baud = 9600

[network]
name = TestUser
port = 9999

[terminal]
mode = vt52
"#;
        let file = create_temp_config(config_content);
        let result = Config::load(file.path());

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::InvalidMode(_)));
    }

    #[test]
    fn test_132_cols_requires_vt220() {
        let config_content = r#"
[serial]
port = /dev/ttyUSB0
baud = 9600

[network]
name = TestUser
port = 9999

[terminal]
mode = vt100
132_cols = true
"#;
        let file = create_temp_config(config_content);
        let result = Config::load(file.path());

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::InvalidColumnsConfig));
    }

    #[test]
    fn test_name_truncation() {
        let config_content = r#"
[serial]
port = /dev/ttyUSB0
baud = 9600

[network]
name = ThisIsAVeryLongUsernameThatExceeds16Characters
port = 9999
"#;
        let file = create_temp_config(config_content);
        let config = Config::load(file.path()).unwrap();

        assert_eq!(config.network.name.chars().count(), 16);
    }

    #[test]
    fn test_missing_file() {
        let result = Config::load("/nonexistent/path/config.ini");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::Io { .. }));
    }
}
