//! UPnP port forwarding support.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

/// Attempt to set up UPnP port forwarding with verbose output
pub fn setup_port_forward(
    local_port: u16,
    external_port: u16,
    description: &str,
    bind_ip: Option<&str>,
) -> Result<SocketAddrV4, super::NetworkError> {
    // Get our local IP - use configured or auto-detect
    let local_ip = match bind_ip {
        Some(ip_str) => ip_str.parse::<Ipv4Addr>().map_err(|e| {
            super::NetworkError::Upnp(format!("Invalid bind_ip '{}': {}", ip_str, e))
        })?,
        None => get_local_ip()?,
    };
    eprintln!("  Local IP: {}", local_ip);

    // Search for gateway with timeout, binding to our local IP
    eprintln!("  Searching for UPnP gateway...");
    let gateway = igd_next::search_gateway(igd_next::SearchOptions {
        timeout: Some(Duration::from_secs(10)),
        bind_addr: SocketAddr::new(IpAddr::V4(local_ip), 0),
        ..Default::default()
    })
    .map_err(|e| super::NetworkError::Upnp(format!("Failed to find gateway: {}", e)))?;

    eprintln!("  Found gateway at: {}", gateway.addr);

    let local_addr = std::net::SocketAddr::V4(SocketAddrV4::new(local_ip, local_port));

    // Request port mapping
    // Lease duration of 0 means permanent (until router restart)
    // We use a reasonable lease time instead
    let lease_duration = 3600; // 1 hour

    eprintln!(
        "  Requesting port mapping: {} -> {}:{}",
        external_port, local_ip, local_port
    );
    gateway
        .add_port(
            igd_next::PortMappingProtocol::UDP,
            external_port,
            local_addr,
            lease_duration,
            description,
        )
        .map_err(|e| super::NetworkError::Upnp(format!("Failed to add port mapping: {}", e)))?;

    // Return the external address
    let external_ip = gateway
        .get_external_ip()
        .map_err(|e| super::NetworkError::Upnp(format!("Failed to get external IP: {}", e)))?;

    match external_ip {
        std::net::IpAddr::V4(ip) => Ok(SocketAddrV4::new(ip, external_port)),
        std::net::IpAddr::V6(_) => Err(super::NetworkError::Upnp(
            "Gateway returned IPv6 address".to_string(),
        )),
    }
}

/// Get the local IP address to use for UPnP
fn get_local_ip() -> Result<std::net::Ipv4Addr, super::NetworkError> {
    // Create a UDP socket and "connect" to a public address
    // This doesn't actually send data but lets us find our local IP
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| super::NetworkError::Upnp(format!("Failed to create socket: {}", e)))?;

    socket
        .connect("8.8.8.8:80")
        .map_err(|e| super::NetworkError::Upnp(format!("Failed to connect socket: {}", e)))?;

    let local_addr = socket
        .local_addr()
        .map_err(|e| super::NetworkError::Upnp(format!("Failed to get local addr: {}", e)))?;

    match local_addr {
        std::net::SocketAddr::V4(addr) => Ok(*addr.ip()),
        std::net::SocketAddr::V6(_) => Err(super::NetworkError::Upnp(
            "IPv6 not supported for UPnP".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_get_local_ip() {
        let ip = super::get_local_ip();
        assert!(ip.is_ok(), "Failed to get local IP: {:?}", ip);
        let ip = ip.unwrap();
        println!("Local IP: {}", ip);
        assert!(!ip.is_loopback());
    }
}
