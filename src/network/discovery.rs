//! Peer discovery using UDP broadcast for LAN discovery.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Discovery broadcast port
pub const DISCOVERY_PORT: u16 = 7891;

/// How often to send discovery announcements
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(10);

/// How long before a peer is considered stale
pub const PEER_TIMEOUT: Duration = Duration::from_secs(30);

/// Magic bytes to identify wormhole discovery packets
const MAGIC: &[u8; 8] = b"ACMSWRMH";

/// Discovery message types
#[derive(Debug, Clone)]
pub enum DiscoveryMessage {
    /// Announce presence on the network
    Announce { name: String, port: u16 },
    /// Request all peers to announce themselves
    Query,
}

impl DiscoveryMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(MAGIC);

        match self {
            DiscoveryMessage::Announce { name, port } => {
                buf.push(0x01);
                buf.extend(port.to_be_bytes());
                buf.push(name.len() as u8);
                buf.extend(name.as_bytes());
            }
            DiscoveryMessage::Query => {
                buf.push(0x02);
            }
        }
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 9 || &data[0..8] != MAGIC {
            return None;
        }

        match data[8] {
            0x01 => {
                // Announce
                if data.len() < 12 {
                    return None;
                }
                let port = u16::from_be_bytes([data[9], data[10]]);
                let name_len = data[11] as usize;
                if data.len() < 12 + name_len {
                    return None;
                }
                let name = String::from_utf8_lossy(&data[12..12 + name_len]).to_string();
                Some(DiscoveryMessage::Announce { name, port })
            }
            0x02 => Some(DiscoveryMessage::Query),
            _ => None,
        }
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
}

impl Discovery {
    /// Create a new discovery service
    pub async fn new(name: String, listen_port: u16) -> Result<Self, super::NetworkError> {
        // Bind to the discovery port with SO_REUSEADDR and SO_REUSEPORT
        // This allows multiple processes on the same machine to share the port
        let socket = {
            // Create socket with socket2 for better control
            let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DISCOVERY_PORT);
            let socket2 = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::DGRAM,
                Some(socket2::Protocol::UDP),
            )
            .map_err(|e| super::NetworkError::Bind(format!("Socket creation failed: {}", e)))?;

            // Set SO_REUSEADDR
            socket2
                .set_reuse_address(true)
                .map_err(|e| super::NetworkError::Bind(format!("SO_REUSEADDR failed: {}", e)))?;

            // Set SO_REUSEPORT using libc directly (socket2 may not expose it)
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let fd = socket2.as_raw_fd();
                let optval: libc::c_int = 1;
                let result = unsafe {
                    libc::setsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_REUSEPORT,
                        &optval as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                    )
                };
                if result != 0 {
                    return Err(super::NetworkError::Bind(format!(
                        "SO_REUSEPORT failed: {}",
                        std::io::Error::last_os_error()
                    )));
                }
            }

            // Set broadcast
            socket2
                .set_broadcast(true)
                .map_err(|e| super::NetworkError::Bind(format!("SO_BROADCAST failed: {}", e)))?;

            // Bind to the discovery port
            socket2
                .bind(&addr.into())
                .map_err(|e| super::NetworkError::Bind(format!("Discovery bind failed: {}", e)))?;

            // Set non-blocking for async
            socket2
                .set_nonblocking(true)
                .map_err(|e| super::NetworkError::Bind(format!("Non-blocking failed: {}", e)))?;

            // Convert to tokio socket
            let std_socket: std::net::UdpSocket = socket2.into();
            tokio::net::UdpSocket::from_std(std_socket).map_err(|e| {
                super::NetworkError::Bind(format!("Tokio socket conversion failed: {}", e))
            })?
        };

        Ok(Self {
            socket: Arc::new(socket),
            our_name: name,
            our_port: listen_port,
        })
    }

    /// Send an announcement to the broadcast address
    pub async fn announce(&self) -> Result<(), super::NetworkError> {
        let msg = DiscoveryMessage::Announce {
            name: self.our_name.clone(),
            port: self.our_port,
        };
        let data = msg.to_bytes();

        // Broadcast to all interfaces
        let broadcast_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), DISCOVERY_PORT);

        self.socket
            .send_to(&data, broadcast_addr)
            .await
            .map_err(|e| super::NetworkError::Send(format!("Broadcast failed: {}", e)))?;

        Ok(())
    }

    /// Send a query to request peers to announce themselves
    pub async fn query(&self) -> Result<(), super::NetworkError> {
        let msg = DiscoveryMessage::Query;
        let data = msg.to_bytes();

        let broadcast_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), DISCOVERY_PORT);

        self.socket
            .send_to(&data, broadcast_addr)
            .await
            .map_err(|e| super::NetworkError::Send(format!("Query broadcast failed: {}", e)))?;

        Ok(())
    }

    /// Get the socket for use in select!
    pub fn socket(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// Get our name
    pub fn name(&self) -> &str {
        &self.our_name
    }

    /// Get our port
    pub fn port(&self) -> u16 {
        self.our_port
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
    let our_name = discovery.name().to_string();
    let _our_port = discovery.port();

    // Initial query to find existing peers
    let _ = discovery.query().await;

    let mut buf = [0u8; 256];

    loop {
        tokio::select! {
            // Periodic announcements
            _ = announce_interval.tick() => {
                if let Err(e) = discovery.announce().await {
                    eprintln!("Discovery announce error: {}", e);
                }
            }

            // Receive discovery messages
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((len, addr)) => {
                        if len == 0 {
                            // Prevent busy loop on 0-byte packets
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                        if let Some(msg) = DiscoveryMessage::from_bytes(&buf[..len]) {
                            match msg {
                                DiscoveryMessage::Announce { name, port } => {
                                    // Ignore our own announcements
                                    if name != our_name {
                                        // Build the peer's actual address (use their IP, their port)
                                        let peer_addr = SocketAddr::new(addr.ip(), port);
                                        let peer = DiscoveredPeer {
                                            name,
                                            addr: peer_addr,
                                        };
                                        let _ = peer_tx.send(peer).await;
                                    }
                                }
                                DiscoveryMessage::Query => {
                                    // Someone is looking for peers, announce ourselves
                                    let _ = discovery.announce().await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Discovery recv error: {}", e);
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
        let msg = DiscoveryMessage::Announce {
            name: "test-node".to_string(),
            port: 7890,
        };
        let bytes = msg.to_bytes();
        let decoded = DiscoveryMessage::from_bytes(&bytes).unwrap();

        match decoded {
            DiscoveryMessage::Announce { name, port } => {
                assert_eq!(name, "test-node");
                assert_eq!(port, 7890);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_query_roundtrip() {
        let msg = DiscoveryMessage::Query;
        let bytes = msg.to_bytes();
        let decoded = DiscoveryMessage::from_bytes(&bytes).unwrap();

        match decoded {
            DiscoveryMessage::Query => {}
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"XXXX\x01\x00\x10\x04test";
        assert!(DiscoveryMessage::from_bytes(data).is_none());
    }
}
