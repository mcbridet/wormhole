//! STUN client for NAT traversal and public endpoint discovery.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use bytecodec::{DecodeExt, EncodeExt};
use stun_codec::rfc5389::methods::BINDING;
use stun_codec::{Message, MessageClass, MessageDecoder, MessageEncoder, TransactionId};

/// Public STUN servers to try
const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun2.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Discover our public IP and port using STUN
/// Note: This uses an ephemeral port for the STUN query, so the returned port
/// may differ from the actual listening port. The public IP is the main value here.
pub fn discover_public_endpoint(_local_port: u16) -> Result<SocketAddr, super::NetworkError> {
    // Bind to an ephemeral port (0) to avoid conflicts with our main socket
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| super::NetworkError::Stun(format!("Failed to bind socket: {}", e)))?;

    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| super::NetworkError::Stun(format!("Failed to set timeout: {}", e)))?;

    // Try each STUN server until one works
    for server in STUN_SERVERS {
        if let Ok(addr) = try_stun_server(&socket, server) {
            return Ok(addr);
        }
    }

    Err(super::NetworkError::Stun(
        "All STUN servers failed".to_string(),
    ))
}

fn try_stun_server(socket: &UdpSocket, server: &str) -> Result<SocketAddr, super::NetworkError> {
    // Resolve server address
    let server_addr = server
        .to_socket_addrs()
        .map_err(|e| super::NetworkError::Stun(format!("DNS resolution failed: {}", e)))?
        .next()
        .ok_or_else(|| super::NetworkError::Stun("No address for STUN server".to_string()))?;

    // Create STUN binding request
    let transaction_id = TransactionId::new(rand::random());
    let request = Message::<stun_codec::rfc5389::Attribute>::new(
        MessageClass::Request,
        BINDING,
        transaction_id,
    );

    // Encode the request
    let mut encoder = MessageEncoder::new();
    let request_bytes = encoder
        .encode_into_bytes(request)
        .map_err(|e| super::NetworkError::Stun(format!("Failed to encode request: {}", e)))?;

    // Send request
    socket
        .send_to(&request_bytes, server_addr)
        .map_err(|e| super::NetworkError::Stun(format!("Failed to send: {}", e)))?;

    // Receive response
    let mut buf = [0u8; 1024];
    let (len, _) = socket
        .recv_from(&mut buf)
        .map_err(|e| super::NetworkError::Stun(format!("Failed to receive: {}", e)))?;

    // Decode response
    let mut decoder = MessageDecoder::<stun_codec::rfc5389::Attribute>::new();
    let response = decoder
        .decode_from_bytes(&buf[..len])
        .map_err(|e| super::NetworkError::Stun(format!("Failed to decode response: {}", e)))?
        .map_err(|e| super::NetworkError::Stun(format!("Incomplete response: {:?}", e)))?;

    // Check transaction ID matches
    if response.transaction_id() != transaction_id {
        return Err(super::NetworkError::Stun(
            "Transaction ID mismatch".to_string(),
        ));
    }

    // Extract mapped address from response
    for attr in response.attributes() {
        if let stun_codec::rfc5389::Attribute::XorMappedAddress(xma) = attr {
            return Ok(xma.address());
        }
    }

    // Try regular MappedAddress as fallback
    for attr in response.attributes() {
        if let stun_codec::rfc5389::Attribute::MappedAddress(ma) = attr {
            return Ok(ma.address());
        }
    }

    Err(super::NetworkError::Stun(
        "No mapped address in response".to_string(),
    ))
}
