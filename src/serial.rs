//! Serial port communication module.

use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::io::{self, Read, Write};
use std::time::Duration;

use crate::config::SerialConfig;

/// Default timeout for serial port operations
const DEFAULT_TIMEOUT_MS: u64 = 100;

/// A wrapper around a serial port connection with reconnection support
pub struct Serial {
    port: Option<Box<dyn SerialPort>>,
    config: SerialConfig,
}

impl Serial {
    /// Open a serial port with the given configuration
    pub fn open(config: &SerialConfig) -> Result<Self, SerialError> {
        let port = Self::open_port(config)?;
        Ok(Self {
            port: Some(port),
            config: config.clone(),
        })
    }

    /// Internal helper to open the port
    fn open_port(config: &SerialConfig) -> Result<Box<dyn SerialPort>, SerialError> {
        serialport::new(&config.port, config.baud_rate)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
            .open()
            .map_err(|e| SerialError::Open {
                port: config.port.clone(),
                source: e,
            })
    }

    /// Check if the serial port is currently connected
    pub fn is_connected(&self) -> bool {
        self.port.is_some()
    }

    /// Attempt to reconnect to the serial port
    pub fn reconnect(&mut self) -> Result<(), SerialError> {
        // Close existing port if any
        self.port = None;
        
        // Try to reopen
        let port = Self::open_port(&self.config)?;
        self.port = Some(port);
        Ok(())
    }

    /// Mark the port as disconnected (after an error)
    pub fn mark_disconnected(&mut self) {
        self.port = None;
    }

    /// Write a string to the serial port
    pub fn write_str(&mut self, s: &str) -> Result<(), SerialError> {
        let port = self.port.as_mut().ok_or(SerialError::Disconnected)?;
        port.write_all(s.as_bytes()).map_err(SerialError::Write)?;
        port.flush().map_err(SerialError::Write)?;
        Ok(())
    }

    /// Write formatted output to the serial port
    pub fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> Result<(), SerialError> {
        let s = std::fmt::format(args);
        self.write_str(&s)
    }

    /// Read available bytes from the serial port (non-blocking style with timeout)
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, SerialError> {
        let port = self.port.as_mut().ok_or(SerialError::Disconnected)?;
        match port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(SerialError::Read(e)),
        }
    }

    /// Read a single byte if available
    pub fn read_byte(&mut self) -> Result<Option<u8>, SerialError> {
        let mut buf = [0u8; 1];
        match self.read(&mut buf)? {
            0 => Ok(None),
            _ => Ok(Some(buf[0])),
        }
    }

    /// Get mutable access to the underlying serial port
    pub fn inner_mut(&mut self) -> Option<&mut Box<dyn SerialPort>> {
        self.port.as_mut()
    }
    
    /// Get the port path
    pub fn port_path(&self) -> &str {
        &self.config.port
    }
}

impl Write for Serial {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.port.as_mut() {
            Some(port) => port.write(buf),
            None => Err(io::Error::new(io::ErrorKind::NotConnected, "serial port disconnected")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.port.as_mut() {
            Some(port) => port.flush(),
            None => Err(io::Error::new(io::ErrorKind::NotConnected, "serial port disconnected")),
        }
    }
}

impl Read for Serial {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.port.as_mut() {
            Some(port) => port.read(buf),
            None => Err(io::Error::new(io::ErrorKind::NotConnected, "serial port disconnected")),
        }
    }
}

#[derive(Debug)]
pub enum SerialError {
    Open {
        port: String,
        source: serialport::Error,
    },
    Write(io::Error),
    Read(io::Error),
    Disconnected,
}

impl SerialError {
    /// Check if this error indicates a disconnection
    pub fn is_disconnect(&self) -> bool {
        match self {
            SerialError::Disconnected => true,
            SerialError::Read(e) | SerialError::Write(e) => {
                matches!(
                    e.kind(),
                    io::ErrorKind::BrokenPipe
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::NotConnected
                        | io::ErrorKind::PermissionDenied
                )
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for SerialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SerialError::Open { port, source } => {
                write!(f, "failed to open serial port '{}': {}", port, source)
            }
            SerialError::Write(e) => write!(f, "serial write error: {}", e),
            SerialError::Read(e) => write!(f, "serial read error: {}", e),
            SerialError::Disconnected => write!(f, "serial port disconnected"),
        }
    }
}

impl std::error::Error for SerialError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SerialError::Open { source, .. } => Some(source),
            SerialError::Write(e) => Some(e),
            SerialError::Read(e) => Some(e),
            SerialError::Disconnected => None,
        }
    }
}
