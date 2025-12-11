//! STUN client for NAT traversal and public address discovery

use anyhow::Result;
use std::net::{SocketAddr, UdpSocket, ToSocketAddrs};
use std::time::Duration;

/// STUN message types
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;

/// STUN attribute types
const STUN_ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const STUN_ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// STUN magic cookie (RFC 5389)
const STUN_MAGIC_COOKIE: u32 = 0x2112A442;

/// Public STUN servers for address discovery
const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun2.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Discover public IP address using STUN
/// Returns the public address as seen by STUN servers
pub fn discover_public_address() -> Result<Option<SocketAddr>> {
    // Try each STUN server until one works
    for server in STUN_SERVERS {
        match query_stun_server(server) {
            Ok(addr) => {
                println!("[STUN] Discovered public address: {} via {}", addr, server);
                return Ok(Some(addr));
            }
            Err(e) => {
                println!("[STUN] Server {} failed: {}", server, e);
                continue;
            }
        }
    }

    println!("[STUN] All STUN servers failed, could not discover public address");
    Ok(None)
}

/// Query a single STUN server for our public address
fn query_stun_server(server: &str) -> Result<SocketAddr> {
    // Resolve server address
    let server_addr = server
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve STUN server"))?;

    // Create UDP socket
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.set_write_timeout(Some(Duration::from_secs(3)))?;

    // Build STUN binding request
    let request = build_binding_request();

    // Send request
    socket.send_to(&request, server_addr)?;

    // Receive response
    let mut buf = [0u8; 1024];
    let (len, _) = socket.recv_from(&mut buf)?;

    // Parse response
    parse_binding_response(&buf[..len])
}

/// Build a STUN binding request
fn build_binding_request() -> Vec<u8> {
    let mut request = Vec::with_capacity(20);

    // Message type (Binding Request)
    request.extend_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());

    // Message length (0 - no attributes)
    request.extend_from_slice(&0u16.to_be_bytes());

    // Magic cookie
    request.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());

    // Transaction ID (12 random bytes)
    let transaction_id: [u8; 12] = rand::random();
    request.extend_from_slice(&transaction_id);

    request
}

/// Parse a STUN binding response and extract the mapped address
fn parse_binding_response(data: &[u8]) -> Result<SocketAddr> {
    if data.len() < 20 {
        anyhow::bail!("STUN response too short");
    }

    // Check message type
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    if msg_type != STUN_BINDING_RESPONSE {
        anyhow::bail!("Not a binding response: 0x{:04x}", msg_type);
    }

    // Get message length
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    if data.len() < 20 + msg_len {
        anyhow::bail!("STUN response truncated");
    }

    // Parse attributes
    let mut pos = 20;
    while pos + 4 <= 20 + msg_len {
        let attr_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let attr_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if pos + attr_len > data.len() {
            break;
        }

        match attr_type {
            STUN_ATTR_XOR_MAPPED_ADDRESS => {
                return parse_xor_mapped_address(&data[pos..pos + attr_len]);
            }
            STUN_ATTR_MAPPED_ADDRESS => {
                return parse_mapped_address(&data[pos..pos + attr_len]);
            }
            _ => {}
        }

        // Padding to 4-byte boundary
        pos += (attr_len + 3) & !3;
    }

    anyhow::bail!("No mapped address in STUN response")
}

/// Parse XOR-MAPPED-ADDRESS attribute
fn parse_xor_mapped_address(data: &[u8]) -> Result<SocketAddr> {
    if data.len() < 8 {
        anyhow::bail!("XOR-MAPPED-ADDRESS too short");
    }

    let family = data[1];
    let xor_port = u16::from_be_bytes([data[2], data[3]]);
    let port = xor_port ^ ((STUN_MAGIC_COOKIE >> 16) as u16);

    match family {
        0x01 => {
            // IPv4
            let xor_ip = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let ip = xor_ip ^ STUN_MAGIC_COOKIE;
            let ip_bytes = ip.to_be_bytes();
            let ip_addr = std::net::Ipv4Addr::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
            Ok(SocketAddr::V4(std::net::SocketAddrV4::new(ip_addr, port)))
        }
        0x02 => {
            // IPv6
            if data.len() < 20 {
                anyhow::bail!("XOR-MAPPED-ADDRESS IPv6 too short");
            }
            // XOR with magic cookie + transaction ID (we don't have it here, so use MAPPED-ADDRESS fallback)
            anyhow::bail!("IPv6 XOR-MAPPED-ADDRESS not implemented")
        }
        _ => anyhow::bail!("Unknown address family: {}", family),
    }
}

/// Parse MAPPED-ADDRESS attribute (fallback)
fn parse_mapped_address(data: &[u8]) -> Result<SocketAddr> {
    if data.len() < 8 {
        anyhow::bail!("MAPPED-ADDRESS too short");
    }

    let family = data[1];
    let port = u16::from_be_bytes([data[2], data[3]]);

    match family {
        0x01 => {
            // IPv4
            let ip_addr = std::net::Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            Ok(SocketAddr::V4(std::net::SocketAddrV4::new(ip_addr, port)))
        }
        0x02 => {
            // IPv6
            if data.len() < 20 {
                anyhow::bail!("MAPPED-ADDRESS IPv6 too short");
            }
            let octets: [u8; 16] = data[4..20].try_into()?;
            let ip_addr = std::net::Ipv6Addr::from(octets);
            Ok(SocketAddr::V6(std::net::SocketAddrV6::new(ip_addr, port, 0, 0)))
        }
        _ => anyhow::bail!("Unknown address family: {}", family),
    }
}

/// Get local address for P2P (LAN connections)
pub fn get_local_address() -> Result<Option<SocketAddr>> {
    // Create a UDP socket and "connect" to a public address to determine local IP
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let local_addr = socket.local_addr()?;

    // Return the local IP with the same port we bound to
    Ok(Some(local_addr))
}

/// Async wrapper for STUN discovery
pub async fn discover_public_address_async() -> Result<Option<SocketAddr>> {
    tokio::task::spawn_blocking(discover_public_address).await?
}

/// Async wrapper for local address discovery
pub async fn get_local_address_async() -> Result<Option<SocketAddr>> {
    tokio::task::spawn_blocking(get_local_address).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_binding_request() {
        let request = build_binding_request();
        assert_eq!(request.len(), 20);
        assert_eq!(request[0], 0x00);
        assert_eq!(request[1], 0x01); // Binding request
    }
}
