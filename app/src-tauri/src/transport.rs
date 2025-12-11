//! Abstract transport layer for unified Relay and P2P connections

use anyhow::Result;
use async_trait::async_trait;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::protocol::{Channel, Frame};

/// Connection type indicator
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionType {
    Relay,
    P2P,
}

impl std::fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionType::Relay => write!(f, "Relay"),
            ConnectionType::P2P => write!(f, "P2P"),
        }
    }
}

/// Abstract transport trait for both Relay and P2P connections
/// Currently used for type abstraction; will be fully utilized when P2P transport is integrated
#[allow(dead_code)]
#[async_trait]
pub trait Transport: Send + Sync {
    /// Read a frame from the transport
    async fn read_frame(&mut self) -> Result<Frame>;

    /// Write a frame to the transport
    async fn write_frame(&mut self, frame: Frame) -> Result<()>;

    /// Get the connection type (Relay or P2P)
    fn connection_type(&self) -> ConnectionType;

    /// Shutdown the transport gracefully
    async fn shutdown(&mut self) -> Result<()>;

    /// Get remote address (for diagnostics, not logging)
    fn remote_addr(&self) -> Option<SocketAddr>;
}

/// Relay transport - wraps TLS stream to relay server
/// Currently relay connections use TlsStream directly; this wrapper enables future abstraction
#[allow(dead_code)]
pub struct RelayTransport {
    stream: TlsStream<TcpStream>,
}

#[allow(dead_code)]
impl RelayTransport {
    pub fn new(stream: TlsStream<TcpStream>) -> Self {
        Self { stream }
    }

    pub fn into_inner(self) -> TlsStream<TcpStream> {
        self.stream
    }

    pub fn inner_mut(&mut self) -> &mut TlsStream<TcpStream> {
        &mut self.stream
    }
}

#[async_trait]
impl Transport for RelayTransport {
    async fn read_frame(&mut self) -> Result<Frame> {
        let mut header = [0u8; 4];
        self.stream.read_exact(&mut header).await?;

        let channel = Channel::try_from(header[0])?;
        let len = ((header[1] as usize) << 16)
            | ((header[2] as usize) << 8)
            | (header[3] as usize);

        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload).await?;

        Ok(Frame::new(channel, payload))
    }

    async fn write_frame(&mut self, frame: Frame) -> Result<()> {
        let len = frame.payload.len();
        let header = [
            frame.channel as u8,
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
        ];

        self.stream.write_all(&header).await?;
        self.stream.write_all(&frame.payload).await?;
        self.stream.flush().await?;
        Ok(())
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Relay
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.stream.shutdown().await?;
        Ok(())
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.stream.get_ref().0.peer_addr().ok()
    }
}

/// P2P transport - direct connection
/// Used when P2P connection succeeds; provides same interface as relay
#[allow(dead_code)]
pub struct P2PTransport {
    pub stream: TcpStream,
    pub remote: SocketAddr,
}

#[allow(dead_code)]
impl P2PTransport {
    pub fn new(stream: TcpStream, remote: SocketAddr) -> Self {
        Self { stream, remote }
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

#[async_trait]
impl Transport for P2PTransport {
    async fn read_frame(&mut self) -> Result<Frame> {
        let mut header = [0u8; 4];
        self.stream.read_exact(&mut header).await?;

        let channel = Channel::try_from(header[0])?;
        let len = ((header[1] as usize) << 16)
            | ((header[2] as usize) << 8)
            | (header[3] as usize);

        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload).await?;

        Ok(Frame::new(channel, payload))
    }

    async fn write_frame(&mut self, frame: Frame) -> Result<()> {
        let len = frame.payload.len();
        let header = [
            frame.channel as u8,
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
        ];

        self.stream.write_all(&header).await?;
        self.stream.write_all(&frame.payload).await?;
        self.stream.flush().await?;
        Ok(())
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::P2P
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.stream.shutdown().await?;
        Ok(())
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.remote)
    }
}

/// P2P connection info exchanged during signaling
#[derive(Debug, Clone)]
pub struct P2PInfo {
    /// Public IP address (from STUN)
    pub public_addr: Option<SocketAddr>,
    /// Local IP address (for LAN connections)
    pub local_addr: Option<SocketAddr>,
    /// Whether P2P is enabled on this side
    pub p2p_enabled: bool,
}

impl P2PInfo {
    pub fn new(public_addr: Option<SocketAddr>, local_addr: Option<SocketAddr>, p2p_enabled: bool) -> Self {
        Self {
            public_addr,
            local_addr,
            p2p_enabled,
        }
    }

    /// Encode P2P info for protocol transmission
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // P2P enabled flag
        data.push(self.p2p_enabled as u8);

        // Public address (if available)
        if let Some(addr) = self.public_addr {
            data.push(1); // Has public addr
            match addr {
                SocketAddr::V4(v4) => {
                    data.push(4); // IPv4
                    data.extend_from_slice(&v4.ip().octets());
                    data.extend_from_slice(&v4.port().to_be_bytes());
                }
                SocketAddr::V6(v6) => {
                    data.push(6); // IPv6
                    data.extend_from_slice(&v6.ip().octets());
                    data.extend_from_slice(&v6.port().to_be_bytes());
                }
            }
        } else {
            data.push(0); // No public addr
        }

        // Local address (if available)
        if let Some(addr) = self.local_addr {
            data.push(1); // Has local addr
            match addr {
                SocketAddr::V4(v4) => {
                    data.push(4); // IPv4
                    data.extend_from_slice(&v4.ip().octets());
                    data.extend_from_slice(&v4.port().to_be_bytes());
                }
                SocketAddr::V6(v6) => {
                    data.push(6); // IPv6
                    data.extend_from_slice(&v6.ip().octets());
                    data.extend_from_slice(&v6.port().to_be_bytes());
                }
            }
        } else {
            data.push(0); // No local addr
        }

        data
    }

    /// Decode P2P info from protocol data
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            anyhow::bail!("Empty P2P info");
        }

        let mut pos = 0;

        // P2P enabled flag
        let p2p_enabled = data[pos] != 0;
        pos += 1;

        // Public address
        let public_addr = if pos < data.len() && data[pos] == 1 {
            pos += 1;
            Some(Self::decode_addr(&data[pos..])?)
        } else {
            if pos < data.len() { pos += 1; }
            None
        };

        // Skip past public address bytes
        if public_addr.is_some() {
            pos += if data.get(pos - 1) == Some(&4) { 6 } else { 18 };
        }

        // Local address
        let local_addr = if pos < data.len() && data[pos] == 1 {
            pos += 1;
            Some(Self::decode_addr(&data[pos..])?)
        } else {
            None
        };

        Ok(Self {
            public_addr,
            local_addr,
            p2p_enabled,
        })
    }

    fn decode_addr(data: &[u8]) -> Result<SocketAddr> {
        if data.is_empty() {
            anyhow::bail!("No address data");
        }

        match data[0] {
            4 => {
                if data.len() < 7 {
                    anyhow::bail!("Invalid IPv4 address");
                }
                let ip = std::net::Ipv4Addr::new(data[1], data[2], data[3], data[4]);
                let port = u16::from_be_bytes([data[5], data[6]]);
                Ok(SocketAddr::V4(std::net::SocketAddrV4::new(ip, port)))
            }
            6 => {
                if data.len() < 19 {
                    anyhow::bail!("Invalid IPv6 address");
                }
                let octets: [u8; 16] = data[1..17].try_into()?;
                let ip = std::net::Ipv6Addr::from(octets);
                let port = u16::from_be_bytes([data[17], data[18]]);
                Ok(SocketAddr::V6(std::net::SocketAddrV6::new(ip, port, 0, 0)))
            }
            _ => anyhow::bail!("Invalid IP version"),
        }
    }
}
