//! Networking module for peer-to-peer communication.
//!
//! Uses UDP for low-latency messaging with STUN for NAT traversal
//! and UPnP for port forwarding when available.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

mod discovery;
mod stun;
mod upnp;

pub use discovery::{run_discovery, DiscoveredPeer, Discovery, PEER_TIMEOUT};
pub use stun::discover_public_endpoint;
pub use upnp::setup_port_forward;

/// Default port for wormhole P2P communication
pub const DEFAULT_PORT: u16 = 7890;

/// Message types for the protocol
#[derive(Debug, Clone)]
pub enum Message {
    /// Text chat message
    Chat { from: String, text: String },
    /// Ping to check connectivity
    Ping { seq: u32 },
    /// Pong response
    Pong { seq: u32 },
    /// Join notification
    Join { name: String },
    /// Leave notification
    Leave { name: String },
    /// Stream frame (ASCII art lines)
    StreamFrame { from: String, lines: Vec<String> },
}

impl Message {
    /// Serialize message to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Message::Chat { from, text } => {
                buf.push(0x01);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
                buf.extend((text.len() as u16).to_be_bytes());
                buf.extend(text.as_bytes());
            }
            Message::Ping { seq } => {
                buf.push(0x02);
                buf.extend(seq.to_be_bytes());
            }
            Message::Pong { seq } => {
                buf.push(0x03);
                buf.extend(seq.to_be_bytes());
            }
            Message::Join { name } => {
                buf.push(0x04);
                buf.push(name.len() as u8);
                buf.extend(name.as_bytes());
            }
            Message::Leave { name } => {
                buf.push(0x05);
                buf.push(name.len() as u8);
                buf.extend(name.as_bytes());
            }
            Message::StreamFrame { from, lines } => {
                buf.push(0x06);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
                buf.push(lines.len() as u8);
                for line in lines {
                    buf.extend((line.len() as u16).to_be_bytes());
                    buf.extend(line.as_bytes());
                }
            }
        }
        buf
    }

    /// Deserialize message from bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }

        match data[0] {
            0x01 => {
                // Chat message
                if data.len() < 4 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len + 2 {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();
                let text_len =
                    u16::from_be_bytes([data[2 + from_len], data[3 + from_len]]) as usize;
                if data.len() < 4 + from_len + text_len {
                    return None;
                }
                let text =
                    String::from_utf8_lossy(&data[4 + from_len..4 + from_len + text_len])
                        .to_string();
                Some(Message::Chat { from, text })
            }
            0x02 => {
                // Ping
                if data.len() < 5 {
                    return None;
                }
                let seq = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
                Some(Message::Ping { seq })
            }
            0x03 => {
                // Pong
                if data.len() < 5 {
                    return None;
                }
                let seq = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
                Some(Message::Pong { seq })
            }
            0x04 => {
                // Join
                if data.len() < 2 {
                    return None;
                }
                let name_len = data[1] as usize;
                if data.len() < 2 + name_len {
                    return None;
                }
                let name = String::from_utf8_lossy(&data[2..2 + name_len]).to_string();
                Some(Message::Join { name })
            }
            0x05 => {
                // Leave
                if data.len() < 2 {
                    return None;
                }
                let name_len = data[1] as usize;
                if data.len() < 2 + name_len {
                    return None;
                }
                let name = String::from_utf8_lossy(&data[2..2 + name_len]).to_string();
                Some(Message::Leave { name })
            }
            0x06 => {
                // StreamFrame
                if data.len() < 2 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len + 1 {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();
                
                let mut offset = 2 + from_len;
                let num_lines = data[offset] as usize;
                offset += 1;
                
                let mut lines = Vec::with_capacity(num_lines);
                for _ in 0..num_lines {
                    if data.len() < offset + 2 {
                        return None;
                    }
                    let line_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                    offset += 2;
                    
                    if data.len() < offset + line_len {
                        return None;
                    }
                    let line = String::from_utf8_lossy(&data[offset..offset + line_len]).to_string();
                    lines.push(line);
                    offset += line_len;
                }
                
                Some(Message::StreamFrame { from, lines })
            }
            _ => None,
        }
    }
}

/// Peer connection state
#[derive(Debug, Clone)]
pub struct Peer {
    /// Peer's display name
    pub name: String,
    /// Peer's socket address
    pub addr: SocketAddr,
    /// Last time we heard from this peer
    pub last_seen: std::time::Instant,
}

/// Network node for P2P communication
pub struct NetworkNode {
    socket: Arc<UdpSocket>,
    local_addr: SocketAddr,
    public_addr: Option<SocketAddr>,
    peers: Vec<Peer>,
    name: String,
}

impl NetworkNode {
    /// Create a new network node
    pub async fn new(name: String, port: u16) -> Result<Self, NetworkError> {
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|e| NetworkError::Bind(e.to_string()))?;

        let local_addr = socket
            .local_addr()
            .map_err(|e| NetworkError::Bind(e.to_string()))?;

        Ok(Self {
            socket: Arc::new(socket),
            local_addr,
            public_addr: None,
            peers: Vec::new(),
            name,
        })
    }

    /// Get the local address
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Get the public address (after STUN discovery)
    pub fn public_addr(&self) -> Option<SocketAddr> {
        self.public_addr
    }

    /// Set the public address (from STUN discovery)
    pub fn set_public_addr(&mut self, addr: SocketAddr) {
        self.public_addr = Some(addr);
    }

    /// Add a peer by address
    pub fn add_peer(&mut self, name: String, addr: SocketAddr) {
        // Don't add ourselves
        if Some(addr) == self.public_addr || addr == self.local_addr {
            return;
        }

        // Update existing peer or add new one
        if let Some(peer) = self.peers.iter_mut().find(|p| p.addr == addr) {
            peer.name = name;
            peer.last_seen = std::time::Instant::now();
        } else {
            self.peers.push(Peer {
                name,
                addr,
                last_seen: std::time::Instant::now(),
            });
        }
    }

    /// Remove stale peers (not seen in the given duration)
    /// Returns the list of peers that were pruned (timed out)
    pub fn prune_peers(&mut self, timeout: Duration) -> Vec<Peer> {
        let now = std::time::Instant::now();
        let mut pruned = Vec::new();
        self.peers.retain(|p| {
            if now.duration_since(p.last_seen) >= timeout {
                pruned.push(p.clone());
                false
            } else {
                true
            }
        });
        pruned
    }

    /// Get list of connected peers
    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }

    /// Get the number of connected peers
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Remove a peer by address, returns the removed peer if found
    pub fn remove_peer(&mut self, addr: SocketAddr) -> Option<Peer> {
        if let Some(idx) = self.peers.iter().position(|p| p.addr == addr) {
            Some(self.peers.remove(idx))
        } else {
            None
        }
    }

    /// Check if a peer with the given address exists and is still active (not timed out)
    pub fn has_peer(&self, addr: SocketAddr, timeout: Duration) -> bool {
        let now = std::time::Instant::now();
        self.peers.iter().any(|p| {
            p.addr == addr && now.duration_since(p.last_seen) < timeout
        })
    }

    /// Update the last_seen time for a peer
    pub fn touch_peer(&mut self, addr: SocketAddr) {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.addr == addr) {
            peer.last_seen = std::time::Instant::now();
        }
    }

    /// Send a message to a specific peer
    pub async fn send_to(&self, msg: &Message, addr: SocketAddr) -> Result<(), NetworkError> {
        let data = msg.to_bytes();
        self.socket
            .send_to(&data, addr)
            .await
            .map_err(|e| NetworkError::Send(e.to_string()))?;
        Ok(())
    }

    /// Broadcast a message to all peers
    pub async fn broadcast(&self, msg: &Message) -> Result<(), NetworkError> {
        let data = msg.to_bytes();
        for peer in &self.peers {
            let _ = self.socket.send_to(&data, peer.addr).await;
        }
        Ok(())
    }

    /// Send a chat message to all peers
    pub async fn send_chat(&self, text: &str) -> Result<(), NetworkError> {
        let msg = Message::Chat {
            from: self.name.clone(),
            text: text.to_string(),
        };
        self.broadcast(&msg).await
    }

    /// Receive a message (with timeout)
    pub async fn recv(&self) -> Result<(Message, SocketAddr), NetworkError> {
        let mut buf = [0u8; 2048];
        let (len, addr) = self
            .socket
            .recv_from(&mut buf)
            .await
            .map_err(|e| NetworkError::Recv(e.to_string()))?;

        Message::from_bytes(&buf[..len])
            .ok_or(NetworkError::InvalidMessage)
            .map(|msg| (msg, addr))
    }

    /// Get a clone of the socket for async operations
    pub fn socket(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// Connect to a peer by address
    pub async fn connect_to_peer(&mut self, addr: SocketAddr) -> Result<(), NetworkError> {
        // Send a join message
        let msg = Message::Join {
            name: self.name.clone(),
        };
        self.send_to(&msg, addr).await?;

        // Add peer with unknown name for now
        self.add_peer("unknown".to_string(), addr);
        Ok(())
    }
}

#[derive(Debug)]
pub enum PeerEvent {
    Joined { name: String },
    Left { name: String },
}

#[derive(Debug)]
pub enum NetworkError {
    Bind(String),
    Send(String),
    Recv(String),
    InvalidMessage,
    Stun(String),
    Upnp(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Bind(e) => write!(f, "failed to bind socket: {}", e),
            NetworkError::Send(e) => write!(f, "failed to send: {}", e),
            NetworkError::Recv(e) => write!(f, "failed to receive: {}", e),
            NetworkError::InvalidMessage => write!(f, "invalid message format"),
            NetworkError::Stun(e) => write!(f, "STUN error: {}", e),
            NetworkError::Upnp(e) => write!(f, "UPnP error: {}", e),
        }
    }
}

impl std::error::Error for NetworkError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_roundtrip() {
        let msg = Message::Chat {
            from: "Alice".to_string(),
            text: "Hello, world!".to_string(),
        };
        let bytes = msg.to_bytes();
        let decoded = Message::from_bytes(&bytes).unwrap();
        match decoded {
            Message::Chat { from, text } => {
                assert_eq!(from, "Alice");
                assert_eq!(text, "Hello, world!");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_ping_pong_roundtrip() {
        let ping = Message::Ping { seq: 42 };
        let bytes = ping.to_bytes();
        let decoded = Message::from_bytes(&bytes).unwrap();
        match decoded {
            Message::Ping { seq } => assert_eq!(seq, 42),
            _ => panic!("Wrong message type"),
        }
    }
}
