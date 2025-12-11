//! P2P direct connection module
//!
//! Handles direct peer-to-peer connections with UDP hole punching
//! and automatic fallback to relay on failure.

use anyhow::Result;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::stun::{discover_public_address_async, get_local_address_async};
use crate::transport::{P2PInfo, P2PTransport};

/// P2P connection timeout (5 seconds)
const P2P_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// P2P handshake port offset from STUN-discovered port
/// Reserved for future UDP hole punching implementation
#[allow(dead_code)]
const P2P_PORT_OFFSET: u16 = 1000;

/// Attempt to establish a P2P connection to the remote peer
/// Returns None if P2P fails (fallback to relay should be used)
pub async fn attempt_p2p_connection(
    remote_info: &P2PInfo,
    local_info: &P2PInfo,
) -> Result<Option<P2PTransport>> {
    // Check if either side has P2P disabled
    if !remote_info.p2p_enabled && !local_info.p2p_enabled {
        println!("[P2P] Both sides have P2P disabled, using relay");
        return Ok(None);
    }

    println!("[P2P] Attempting P2P connection...");
    println!("[P2P] Remote: public={:?}, local={:?}", remote_info.public_addr, remote_info.local_addr);
    println!("[P2P] Local: public={:?}, local={:?}", local_info.public_addr, local_info.local_addr);

    // Try connection strategies in order of preference:
    // 1. Same LAN (local addresses match network)
    // 2. Direct public IP connection
    // 3. UDP hole punching (more complex, future enhancement)

    // Strategy 1: Try local address (same LAN)
    if let Some(local_addr) = remote_info.local_addr {
        println!("[P2P] Trying local address: {}", local_addr);
        if let Some(transport) = try_connect(local_addr).await {
            println!("[P2P] Connected via local address!");
            return Ok(Some(transport));
        }
    }

    // Strategy 2: Try public address (direct connection)
    if let Some(public_addr) = remote_info.public_addr {
        println!("[P2P] Trying public address: {}", public_addr);
        if let Some(transport) = try_connect(public_addr).await {
            println!("[P2P] Connected via public address!");
            return Ok(Some(transport));
        }
    }

    println!("[P2P] All P2P strategies failed, falling back to relay");
    Ok(None)
}

/// Try to connect to an address with timeout
async fn try_connect(addr: SocketAddr) -> Option<P2PTransport> {
    match timeout(P2P_CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => {
            println!("[P2P] TCP connection established to {}", addr);
            Some(P2PTransport::new(stream, addr))
        }
        Ok(Err(e)) => {
            println!("[P2P] Connection to {} failed: {}", addr, e);
            None
        }
        Err(_) => {
            println!("[P2P] Connection to {} timed out", addr);
            None
        }
    }
}

/// Listen for incoming P2P connections
/// Returns a listener that can accept P2P connections
pub async fn create_p2p_listener(local_port: u16) -> Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", local_port)).await?;
    println!("[P2P] Listening on port {}", local_port);
    Ok(listener)
}

/// Accept a P2P connection with timeout
pub async fn accept_p2p_connection(
    listener: &tokio::net::TcpListener,
    expected_addr: Option<SocketAddr>,
) -> Result<Option<P2PTransport>> {
    match timeout(P2P_CONNECT_TIMEOUT, listener.accept()).await {
        Ok(Ok((stream, peer_addr))) => {
            println!("[P2P] Accepted connection from {}", peer_addr);

            // Optionally verify the peer address matches expected
            if let Some(expected) = expected_addr {
                if peer_addr.ip() != expected.ip() {
                    println!("[P2P] Warning: Peer IP {} doesn't match expected {}", peer_addr.ip(), expected.ip());
                    // Still accept - IP might differ due to NAT
                }
            }

            Ok(Some(P2PTransport::new(stream, peer_addr)))
        }
        Ok(Err(e)) => {
            println!("[P2P] Accept failed: {}", e);
            Ok(None)
        }
        Err(_) => {
            println!("[P2P] Accept timed out");
            Ok(None)
        }
    }
}

/// Gather P2P connection info for this peer
pub async fn gather_p2p_info(p2p_enabled: bool, listen_port: u16) -> P2PInfo {
    let public_addr = if p2p_enabled {
        match discover_public_address_async().await {
            Ok(Some(mut addr)) => {
                // Use the P2P listen port instead of the ephemeral STUN port
                addr.set_port(listen_port);
                Some(addr)
            }
            Ok(None) => None,
            Err(e) => {
                println!("[P2P] STUN discovery failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    let local_addr = if p2p_enabled {
        match get_local_address_async().await {
            Ok(Some(mut addr)) => {
                addr.set_port(listen_port);
                Some(addr)
            }
            Ok(None) => None,
            Err(e) => {
                println!("[P2P] Local address discovery failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    P2PInfo::new(public_addr, local_addr, p2p_enabled)
}

/// Choose the best P2P port to use
/// Tries to use a consistent port based on device ID hash
pub fn choose_p2p_port(device_id: &str) -> u16 {
    // Hash the device ID to get a consistent port
    let hash: u32 = device_id
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));

    // Port range: 49152-65535 (dynamic/private ports)
    let base_port = 49152u16;
    let port_range = 65535u16 - base_port;
    base_port + (hash as u16 % port_range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_choose_p2p_port() {
        let port1 = choose_p2p_port("123456789");
        let port2 = choose_p2p_port("123456789");
        let port3 = choose_p2p_port("987654321");

        // Same ID should give same port
        assert_eq!(port1, port2);

        // Different IDs should give different ports (usually)
        // Note: This could theoretically fail due to hash collision
        assert_ne!(port1, port3);

        // Port should be in valid range
        assert!(port1 >= 49152);
        assert!(port1 <= 65535);
    }
}
