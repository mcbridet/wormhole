//! Networking module for peer-to-peer communication.
//!
//! Uses UDP for low-latency messaging with STUN for NAT traversal
//! and UPnP for port forwarding when available.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

mod discovery;
mod stun;
mod upnp;

pub use discovery::{DiscoveredPeer, Discovery, PEER_TIMEOUT, run_discovery};
pub use stun::discover_public_endpoint;
pub use upnp::setup_port_forward;

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
    /// Call request
    CallRequest { from: String },
    /// Call hangup notification
    CallHangup { from: String },
    /// Call rejected (busy)
    CallReject { from: String },
    /// Stream frame (ASCII art lines) - deprecated, kept for compatibility
    StreamFrame { from: String, lines: Vec<String> },
    /// Video frame (raw grayscale image data for receiver-side rendering)
    VideoFrame {
        from: String,
        width: u16,
        height: u16,
        pixels: Vec<u8>,
    },
    /// Video frame fragment (for large frames that exceed UDP MTU)
    VideoFrameFragment {
        from: String,
        width: u16,
        height: u16,
        frame_id: u8,        // Unique ID for this frame (wraps around)
        fragment_idx: u8,    // Which fragment this is (0-indexed)
        total_fragments: u8, // Total number of fragments
        data: Vec<u8>,       // Compressed pixel data fragment
    },
    /// Discovery announce (sent to main port as fallback for SO_REUSEPORT issues)
    DiscoveryAnnounce { name: String, port: u16 },
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
            Message::CallRequest { from } => {
                buf.push(0x07);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
            }
            Message::CallHangup { from } => {
                buf.push(0x08);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
            }
            Message::CallReject { from } => {
                buf.push(0x09);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
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
            Message::VideoFrame {
                from,
                width,
                height,
                pixels,
            } => {
                buf.push(0x0A);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
                buf.extend(width.to_be_bytes());
                buf.extend(height.to_be_bytes());
                // Store uncompressed size, then LZ4 compressed data
                buf.extend((pixels.len() as u32).to_be_bytes());
                let compressed = lz4_flex::compress_prepend_size(pixels);
                buf.extend((compressed.len() as u32).to_be_bytes());
                buf.extend(&compressed);
            }
            Message::VideoFrameFragment {
                from,
                width,
                height,
                frame_id,
                fragment_idx,
                total_fragments,
                data,
            } => {
                buf.push(0x0C);
                buf.push(from.len() as u8);
                buf.extend(from.as_bytes());
                buf.extend(width.to_be_bytes());
                buf.extend(height.to_be_bytes());
                buf.push(*frame_id);
                buf.push(*fragment_idx);
                buf.push(*total_fragments);
                buf.extend((data.len() as u32).to_be_bytes());
                buf.extend(data);
            }
            Message::DiscoveryAnnounce { name, port } => {
                buf.push(0x0B);
                buf.extend(port.to_be_bytes());
                buf.push(name.len() as u8);
                buf.extend(name.as_bytes());
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
                let text = String::from_utf8_lossy(&data[4 + from_len..4 + from_len + text_len])
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
            0x07 => {
                // CallRequest
                if data.len() < 2 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();
                Some(Message::CallRequest { from })
            }
            0x08 => {
                // CallHangup
                if data.len() < 2 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();
                Some(Message::CallHangup { from })
            }
            0x09 => {
                // CallReject
                if data.len() < 2 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();
                Some(Message::CallReject { from })
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
                    let line =
                        String::from_utf8_lossy(&data[offset..offset + line_len]).to_string();
                    lines.push(line);
                    offset += line_len;
                }

                Some(Message::StreamFrame { from, lines })
            }
            0x0A => {
                // VideoFrame (LZ4 compressed)
                if data.len() < 2 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len + 12 {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();

                let mut offset = 2 + from_len;
                let width = u16::from_be_bytes([data[offset], data[offset + 1]]);
                offset += 2;
                let height = u16::from_be_bytes([data[offset], data[offset + 1]]);
                offset += 2;
                // Uncompressed size (for validation)
                let _uncompressed_len = u32::from_be_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]) as usize;
                offset += 4;
                // Compressed size
                let compressed_len = u32::from_be_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]) as usize;
                offset += 4;

                if data.len() < offset + compressed_len {
                    return None;
                }
                let compressed = &data[offset..offset + compressed_len];

                // Decompress the pixel data
                let pixels = match lz4_flex::decompress_size_prepended(compressed) {
                    Ok(p) => p,
                    Err(_) => return None,
                };

                Some(Message::VideoFrame {
                    from,
                    width,
                    height,
                    pixels,
                })
            }
            0x0B => {
                // DiscoveryAnnounce
                if data.len() < 4 {
                    return None;
                }
                let port = u16::from_be_bytes([data[1], data[2]]);
                let name_len = data[3] as usize;
                if data.len() < 4 + name_len {
                    return None;
                }
                let name = String::from_utf8_lossy(&data[4..4 + name_len]).to_string();
                Some(Message::DiscoveryAnnounce { name, port })
            }
            0x0C => {
                // VideoFrameFragment
                if data.len() < 2 {
                    return None;
                }
                let from_len = data[1] as usize;
                if data.len() < 2 + from_len + 11 {
                    return None;
                }
                let from = String::from_utf8_lossy(&data[2..2 + from_len]).to_string();

                let mut offset = 2 + from_len;
                let width = u16::from_be_bytes([data[offset], data[offset + 1]]);
                offset += 2;
                let height = u16::from_be_bytes([data[offset], data[offset + 1]]);
                offset += 2;
                let frame_id = data[offset];
                offset += 1;
                let fragment_idx = data[offset];
                offset += 1;
                let total_fragments = data[offset];
                offset += 1;
                let data_len = u32::from_be_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]) as usize;
                offset += 4;

                if data.len() < offset + data_len {
                    return None;
                }
                let frag_data = data[offset..offset + data_len].to_vec();

                Some(Message::VideoFrameFragment {
                    from,
                    width,
                    height,
                    frame_id,
                    fragment_idx,
                    total_fragments,
                    data: frag_data,
                })
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

/// Grace period after a peer leaves before we accept discovery from them again
const LEAVE_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Buffer for reassembling fragmented video frames
#[derive(Debug)]
#[allow(dead_code)]
struct FragmentBuffer {
    from: String,
    width: u16,
    height: u16,
    frame_id: u8,
    total_fragments: u8,
    fragments: Vec<Option<Vec<u8>>>,
    received_at: Instant,
}

impl FragmentBuffer {
    fn new(from: String, width: u16, height: u16, frame_id: u8, total_fragments: u8) -> Self {
        Self {
            from,
            width,
            height,
            frame_id,
            total_fragments,
            fragments: vec![None; total_fragments as usize],
            received_at: Instant::now(),
        }
    }

    fn add_fragment(&mut self, idx: u8, data: Vec<u8>) {
        if (idx as usize) < self.fragments.len() {
            self.fragments[idx as usize] = Some(data);
        }
    }

    fn is_complete(&self) -> bool {
        self.fragments.iter().all(|f| f.is_some())
    }

    fn reassemble(&self) -> Option<Vec<u8>> {
        if !self.is_complete() {
            return None;
        }
        let compressed: Vec<u8> = self
            .fragments
            .iter()
            .filter_map(|f| f.as_ref())
            .flatten()
            .copied()
            .collect();

        // Decompress
        lz4_flex::decompress_size_prepended(&compressed).ok()
    }
}

/// Network node for P2P communication
pub struct NetworkNode {
    socket: Arc<UdpSocket>,
    local_addr: SocketAddr,
    public_addr: Option<SocketAddr>,
    peers: Vec<Peer>,
    /// Set of all peer addresses we've ever connected to (persists across disconnects)
    known_addrs: HashSet<SocketAddr>,
    /// Addresses that recently sent Leave messages (addr -> time of leave)
    recently_left: HashMap<SocketAddr, Instant>,
    name: String,
    /// Fragment buffers for reassembling video frames (keyed by (peer_name, frame_id))
    fragment_buffers: HashMap<(String, u8), FragmentBuffer>,
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
            known_addrs: HashSet::new(),
            recently_left: HashMap::new(),
            name,
            fragment_buffers: HashMap::new(),
        })
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

        // Track this address permanently
        self.known_addrs.insert(addr);

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

    /// Remove a peer by address and record their departure time
    pub fn remove_peer(&mut self, addr: SocketAddr) {
        self.peers.retain(|p| p.addr != addr);
        self.recently_left.insert(addr, Instant::now());
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

    /// Check if a peer with the given address exists and is still active (not timed out)
    pub fn has_peer(&self, addr: SocketAddr, timeout: Duration) -> bool {
        let now = std::time::Instant::now();
        self.peers
            .iter()
            .any(|p| p.addr == addr && now.duration_since(p.last_seen) < timeout)
    }

    /// Check if we've ever connected to a peer at this address
    pub fn knows_peer(&self, addr: SocketAddr) -> bool {
        self.known_addrs.contains(&addr)
    }

    /// Check if a peer recently left (within grace period)
    /// Also cleans up stale entries
    pub fn recently_left(&mut self, addr: SocketAddr) -> bool {
        let now = Instant::now();
        // Clean up old entries
        self.recently_left
            .retain(|_, left_at| now.duration_since(*left_at) < LEAVE_GRACE_PERIOD);
        self.recently_left.contains_key(&addr)
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

    /// Send a video frame, fragmenting if necessary to fit within UDP MTU
    /// Max safe UDP payload is ~60KB, we use 50KB to be conservative
    pub async fn send_video_frame(
        &self,
        from: &str,
        width: u16,
        height: u16,
        pixels: &[u8],
        frame_id: u8,
        addr: SocketAddr,
    ) -> Result<(), NetworkError> {
        // Compress the pixels first
        let compressed = lz4_flex::compress_prepend_size(pixels);

        // Max fragment size - use 1400 bytes to stay under typical MTU (1500)
        // and avoid IP-level fragmentation which causes packet loss
        const MAX_FRAGMENT_SIZE: usize = 1400;

        if compressed.len() <= MAX_FRAGMENT_SIZE {
            // Can send as a single fragment
            let msg = Message::VideoFrameFragment {
                from: from.to_string(),
                width,
                height,
                frame_id,
                fragment_idx: 0,
                total_fragments: 1,
                data: compressed,
            };
            self.send_to(&msg, addr).await
        } else {
            // Need to fragment
            let total_fragments = compressed.len().div_ceil(MAX_FRAGMENT_SIZE);
            if total_fragments > 255 {
                return Err(NetworkError::Send(
                    "Frame too large to fragment".to_string(),
                ));
            }

            for (idx, chunk) in compressed.chunks(MAX_FRAGMENT_SIZE).enumerate() {
                let msg = Message::VideoFrameFragment {
                    from: from.to_string(),
                    width,
                    height,
                    frame_id,
                    fragment_idx: idx as u8,
                    total_fragments: total_fragments as u8,
                    data: chunk.to_vec(),
                };
                self.send_to(&msg, addr).await?;
            }
            Ok(())
        }
    }

    /// Process a video frame fragment. Returns Some(VideoFrame) if the frame is now complete.
    #[allow(clippy::too_many_arguments)]
    pub fn process_fragment(
        &mut self,
        from: String,
        width: u16,
        height: u16,
        frame_id: u8,
        fragment_idx: u8,
        total_fragments: u8,
        data: Vec<u8>,
    ) -> Option<Message> {
        // Clean up old fragment buffers (older than 2 seconds)
        let now = Instant::now();
        self.fragment_buffers
            .retain(|_, buf| now.duration_since(buf.received_at) < Duration::from_secs(2));

        // Key is (peer_name, frame_id) to allow multiple frames to be assembled in parallel
        let key = (from.clone(), frame_id);

        // Get or create buffer for this frame
        let buffer = self.fragment_buffers.entry(key.clone()).or_insert_with(|| {
            FragmentBuffer::new(from.clone(), width, height, frame_id, total_fragments)
        });

        // Add the fragment
        buffer.add_fragment(fragment_idx, data);

        // Check if complete and reassemble
        if buffer.is_complete()
            && let Some(pixels) = buffer.reassemble()
        {
            // Remove the buffer
            self.fragment_buffers.remove(&key);
            return Some(Message::VideoFrame {
                from,
                width,
                height,
                pixels,
            });
        }

        None
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
    Joined { name: String, addr: SocketAddr },
    Left { name: String, addr: SocketAddr },
}

#[derive(Debug)]
pub enum NetworkError {
    Bind(String),
    Send(String),
    Stun(String),
    Upnp(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Bind(e) => write!(f, "failed to bind socket: {}", e),
            NetworkError::Send(e) => write!(f, "failed to send: {}", e),
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

    #[test]
    fn test_video_frame_roundtrip() {
        let frame = Message::VideoFrame {
            from: "Bob".to_string(),
            width: 80,
            height: 44,
            pixels: vec![0, 128, 255, 64, 192],
        };
        let bytes = frame.to_bytes();
        let decoded = Message::from_bytes(&bytes).unwrap();
        match decoded {
            Message::VideoFrame {
                from,
                width,
                height,
                pixels,
            } => {
                assert_eq!(from, "Bob");
                assert_eq!(width, 80);
                assert_eq!(height, 44);
                assert_eq!(pixels, vec![0, 128, 255, 64, 192]);
            }
            _ => panic!("Wrong message type"),
        }
    }
}
