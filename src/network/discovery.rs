//! Peer discovery using UDP broadcast for LAN discovery.
//!
//! This module implements a simple LAN discovery protocol:
//! 1. Each peer periodically broadcasts an Announce message to all subnet broadcast addresses
//! 2. When a peer receives an Announce, it replies directly to the sender
//! 3. Discovered peers are sent to the application via a channel

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Discovery broadcast port
pub const DISCOVERY_PORT: u16 = 7891;

/// How often to send discovery announcements
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(5);

/// How long before a peer is considered stale
pub const PEER_TIMEOUT: Duration = Duration::from_secs(30);

/// Magic bytes to identify wormhole discovery packets
const MAGIC: &[u8; 8] = b"ACMSWRMH";

/// Discovery announcement message
#[derive(Debug, Clone)]
pub struct DiscoveryMessage {
    pub name: String,
    pub port: u16,
}

impl DiscoveryMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(MAGIC);
        buf.extend(self.port.to_be_bytes());
        buf.push(self.name.len() as u8);
        buf.extend(self.name.as_bytes());
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 11 || &data[0..8] != MAGIC {
            return None;
        }
        let port = u16::from_be_bytes([data[8], data[9]]);
        let name_len = data[10] as usize;
        if data.len() < 11 + name_len {
            return None;
        }
        let name = String::from_utf8_lossy(&data[11..11 + name_len]).to_string();
        Some(DiscoveryMessage { name, port })
    }
}

/// Get broadcast address for a specific bind IP, or all if bind_ip is 0.0.0.0
fn get_broadcast_addresses(bind_ip: Ipv4Addr) -> Vec<Ipv4Addr> {
    let mut addrs = Vec::new();

    // If bind_ip is 0.0.0.0, we need to find the right interface(s)
    // If bind_ip is specific, only use that interface's broadcast
    let use_all_interfaces = bind_ip == Ipv4Addr::UNSPECIFIED;

    if let Ok(interfaces) = get_if_addrs::get_if_addrs() {
        for iface in interfaces {
            // Skip loopback
            if iface.is_loopback() {
                continue;
            }

            if let std::net::IpAddr::V4(ipv4) = iface.ip() {
                // If we have a specific bind_ip, only use matching interface
                if !use_all_interfaces && ipv4 != bind_ip {
                    continue;
                }

                // Try to get the broadcast address from the interface
                if let Some(broadcast) = get_broadcast_from_interface(&iface) {
                    if !addrs.contains(&broadcast) {
                        addrs.push(broadcast);
                    }
                } else {
                    // Fallback: compute broadcast assuming /24 subnet
                    let octets = ipv4.octets();
                    let broadcast_24 = Ipv4Addr::new(octets[0], octets[1], octets[2], 255);
                    if !addrs.contains(&broadcast_24) {
                        addrs.push(broadcast_24);
                    }
                }
            }
        }
    }

    // If we didn't find any suitable interface, fall back to limited broadcast
    if addrs.is_empty() {
        addrs.push(Ipv4Addr::BROADCAST);
    }

    // Also include localhost for same-machine peers (useful for testing)
    addrs.push(Ipv4Addr::LOCALHOST);

    addrs
}

/// Extract broadcast address from interface info
fn get_broadcast_from_interface(iface: &get_if_addrs::Interface) -> Option<Ipv4Addr> {
    // get_if_addrs provides broadcast via the IfAddr enum
    match &iface.addr {
        get_if_addrs::IfAddr::V4(v4) => v4.broadcast,
        _ => None,
    }
}

/// Discovered peer information
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub name: String,
    pub addr: SocketAddr,
}

/// Peer discovery service
pub struct Discovery {
    socket: Arc<UdpSocket>,
    our_name: String,
    our_port: u16,
    broadcast_addrs: Vec<Ipv4Addr>,
}

impl Discovery {
    /// Create a new discovery service
    ///
    /// `bind_ip` should match the network config's bind_ip - if it's a specific IP,
    /// discovery will only broadcast on that interface. If it's 0.0.0.0, it will
    /// broadcast on all non-loopback interfaces.
    pub async fn new(
        name: String,
        listen_port: u16,
        bind_ip: Ipv4Addr,
    ) -> Result<Self, super::NetworkError> {
        // Create a simple UDP socket bound to the discovery port
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DISCOVERY_PORT);

        // Use socket2 for SO_REUSEADDR (allows binding even if port is in TIME_WAIT)
        let socket2 = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(|e| super::NetworkError::Bind(format!("Socket creation failed: {}", e)))?;

        socket2
            .set_reuse_address(true)
            .map_err(|e| super::NetworkError::Bind(format!("SO_REUSEADDR failed: {}", e)))?;

        socket2
            .set_broadcast(true)
            .map_err(|e| super::NetworkError::Bind(format!("SO_BROADCAST failed: {}", e)))?;

        socket2
            .bind(&addr.into())
            .map_err(|e| super::NetworkError::Bind(format!("Discovery bind failed: {}", e)))?;

        socket2
            .set_nonblocking(true)
            .map_err(|e| super::NetworkError::Bind(format!("Non-blocking failed: {}", e)))?;

        let std_socket: std::net::UdpSocket = socket2.into();
        let socket = tokio::net::UdpSocket::from_std(std_socket).map_err(|e| {
            super::NetworkError::Bind(format!("Tokio socket conversion failed: {}", e))
        })?;

        // Get broadcast addresses based on bind_ip
        let broadcast_addrs = get_broadcast_addresses(bind_ip);

        Ok(Self {
            socket: Arc::new(socket),
            our_name: name,
            our_port: listen_port,
            broadcast_addrs,
        })
    }

    /// Send an announcement to all broadcast addresses
    pub async fn announce(&self) {
        let msg = DiscoveryMessage {
            name: self.our_name.clone(),
            port: self.our_port,
        };
        let data = msg.to_bytes();

        // Send to all known broadcast addresses (ignore errors)
        for addr in &self.broadcast_addrs {
            let dest = SocketAddr::new(IpAddr::V4(*addr), DISCOVERY_PORT);
            let _ = self.socket.send_to(&data, dest).await;
        }
    }

    /// Send an announcement directly to a specific address (for unicast reply)
    pub async fn announce_to(&self, target: SocketAddr) {
        let msg = DiscoveryMessage {
            name: self.our_name.clone(),
            port: self.our_port,
        };
        let data = msg.to_bytes();
        let _ = self.socket.send_to(&data, target).await;
    }

    /// Get the socket for use in select!
    pub fn socket(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// Get our name
    pub fn name(&self) -> &str {
        &self.our_name
    }
}

/// Run the discovery service, returning discovered peers via channel
pub async fn run_discovery(
    discovery: Arc<Discovery>,
    peer_tx: mpsc::Sender<DiscoveredPeer>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let socket = discovery.socket();
    let mut announce_interval = tokio::time::interval(ANNOUNCE_INTERVAL);
    // Skip the first immediate tick so we control initial timing
    announce_interval.tick().await;

    let our_name = discovery.name().to_string();

    let mut buf = [0u8; 256];

    // Send initial announcement after a tiny delay
    let mut initial_announce_done = false;
    let initial_announce_delay = tokio::time::sleep(Duration::from_millis(100));
    tokio::pin!(initial_announce_delay);

    loop {
        tokio::select! {
            // Initial announcement after a tiny delay (ensures receive loop is active)
            _ = &mut initial_announce_delay, if !initial_announce_done => {
                initial_announce_done = true;
                let _ = discovery.announce().await;
            }

            // Periodic announcements
            _ = announce_interval.tick() => {
                let _ = discovery.announce().await;
            }

            // Receive discovery messages
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((len, addr)) => {
                        if len == 0 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }

                        if let Some(msg) = DiscoveryMessage::from_bytes(&buf[..len]) {
                            // Ignore our own announcements
                            if msg.name == our_name {
                                continue;
                            }

                            // Build the peer's actual address (their IP, their app port)
                            let peer_addr = SocketAddr::new(addr.ip(), msg.port);
                            let peer = DiscoveredPeer {
                                name: msg.name,
                                addr: peer_addr,
                            };
                            let _ = peer_tx.send(peer).await;

                            // Reply directly to the sender's discovery port so they see us too
                            let reply_addr = SocketAddr::new(addr.ip(), DISCOVERY_PORT);
                            discovery.announce_to(reply_addr).await;
                        }
                    }
                    Err(_) => {
                        // Brief pause on receive errors to avoid busy loop
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }

            // Shutdown signal
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announce_roundtrip() {
        let msg = DiscoveryMessage {
            name: "test-node".to_string(),
            port: 7890,
        };
        let bytes = msg.to_bytes();
        let decoded = DiscoveryMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.name, "test-node");
        assert_eq!(decoded.port, 7890);
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"XXXX\x01\x00\x10\x04test";
        assert!(DiscoveryMessage::from_bytes(data).is_none());
    }
}
