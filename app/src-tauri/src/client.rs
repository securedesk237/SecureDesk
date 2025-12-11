//! Client mode - connect to remote PCs

#![allow(dead_code)]

use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::crypto::{Identity, SecureChannel};
use crate::p2p::{attempt_p2p_connection, gather_p2p_info, choose_p2p_port};
use crate::protocol::{self, Channel, Frame};
use crate::transport::{ConnectionType, P2PInfo};

/// Client session - controlling a remote PC
pub struct ClientSession {
    stream: Option<tokio_rustls::client::TlsStream<TcpStream>>,
    p2p_stream: Option<TcpStream>,
    channel: Option<SecureChannel>,
    remote_id: String,
    connection_type: ConnectionType,
}

impl ClientSession {
    /// Connect to remote device via relay (with optional P2P upgrade)
    pub async fn connect(
        relay_address: String,
        remote_id: String,
        identity: Identity,
    ) -> Result<Self> {
        Self::connect_with_p2p(relay_address, remote_id, identity, true).await
    }

    /// Connect to remote device with explicit P2P control
    pub async fn connect_with_p2p(
        relay_address: String,
        remote_id: String,
        identity: Identity,
        p2p_enabled: bool,
    ) -> Result<Self> {
        // Parse address
        let (host, port) = relay_address
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid relay address"))?;
        let port: u16 = port.parse()?;

        // TLS setup
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));

        // Connect to relay
        let tcp = TcpStream::connect(format!("{}:{}", host, port)).await?;
        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_owned())?;
        let mut stream = connector.connect(server_name, tcp).await?;

        // Register as technician wanting to connect to remote_id
        let my_id = identity.device_id_raw();
        let target_id = remote_id.replace(' ', "");

        stream.write_u8(0x02).await?; // Technician type
        // Use big-endian for protocol compatibility with Go server
        stream.write_all(&(my_id.len() as u16).to_be_bytes()).await?;
        stream.write_all(my_id.as_bytes()).await?;
        stream.write_all(&(target_id.len() as u16).to_be_bytes()).await?;
        stream.write_all(target_id.as_bytes()).await?;
        stream.flush().await?;

        // Wait for response from relay server
        // The relay sends a control frame: [channel_id (1)][length (3)][payload]
        // Success: channel=0x00, payload[0]=0x01 (session established)
        // Error: channel=0x00, payload[0]=0xFF followed by error message
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await?;

        let channel = header[0];
        let len = ((header[1] as usize) << 16)
            | ((header[2] as usize) << 8)
            | (header[3] as usize);

        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await?;

        // Check if it's an error response
        if channel == 0x00 && !payload.is_empty() && payload[0] == 0xFF {
            let error_msg = String::from_utf8_lossy(&payload[1..]).to_string();
            anyhow::bail!("Connection failed: {}", error_msg);
        }

        // P2P negotiation (if enabled)
        let mut connection_type = ConnectionType::Relay;
        let mut p2p_stream: Option<TcpStream> = None;

        if p2p_enabled {
            println!("[CLIENT] P2P enabled, gathering P2P info...");
            let p2p_port = choose_p2p_port(&my_id);
            let local_info = gather_p2p_info(p2p_enabled, p2p_port).await;

            // Send P2P offer to host via relay
            let offer_data = local_info.encode();
            let offer_frame = Frame::control(protocol::control::P2P_OFFER, &offer_data);
            Self::write_frame_to_stream(&mut stream, offer_frame).await?;
            println!("[CLIENT] Sent P2P offer");

            // Wait for P2P answer from host
            if let Ok(answer_frame) = Self::read_frame_from_stream(&mut stream).await {
                if answer_frame.channel == Channel::Control
                    && !answer_frame.payload.is_empty()
                    && answer_frame.payload[0] == protocol::control::P2P_ANSWER
                {
                    if let Ok(remote_info) = P2PInfo::decode(&answer_frame.payload[1..]) {
                        println!("[CLIENT] Received P2P answer: {:?}", remote_info);

                        // Attempt P2P connection
                        if let Ok(Some(transport)) = attempt_p2p_connection(&remote_info, &local_info).await {
                            println!("[CLIENT] P2P connection established!");
                            p2p_stream = Some(transport.stream);
                            connection_type = ConnectionType::P2P;

                            // Notify host that P2P is ready
                            let ready_frame = Frame::control(protocol::control::P2P_READY, &[]);
                            Self::write_frame_to_stream(&mut stream, ready_frame).await?;
                        } else {
                            println!("[CLIENT] P2P failed, using relay");
                            let failed_frame = Frame::control(protocol::control::P2P_FAILED, &[]);
                            Self::write_frame_to_stream(&mut stream, failed_frame).await?;
                        }
                    }
                }
            }
        }

        let session = Self {
            stream: Some(stream),
            p2p_stream,
            channel: None,
            remote_id: target_id,
            connection_type,
        };

        Ok(session)
    }

    /// Get the current connection type
    pub fn connection_type(&self) -> ConnectionType {
        self.connection_type
    }

    /// Helper to write frame to stream
    async fn write_frame_to_stream(
        stream: &mut tokio_rustls::client::TlsStream<TcpStream>,
        frame: Frame,
    ) -> Result<()> {
        let len = frame.payload.len();
        let header = [
            frame.channel as u8,
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
        ];
        stream.write_all(&header).await?;
        stream.write_all(&frame.payload).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Helper to read frame from stream
    async fn read_frame_from_stream(
        stream: &mut tokio_rustls::client::TlsStream<TcpStream>,
    ) -> Result<Frame> {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await?;

        let channel = Channel::try_from(header[0])?;
        let len = ((header[1] as usize) << 16)
            | ((header[2] as usize) << 8)
            | (header[3] as usize);

        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await?;

        Ok(Frame::new(channel, payload))
    }

    async fn read_frame(&mut self) -> Result<Frame> {
        let stream = self.stream.as_mut().ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await?;

        let channel = Channel::try_from(header[0])?;
        let len = ((header[1] as usize) << 16)
            | ((header[2] as usize) << 8)
            | (header[3] as usize);

        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await?;

        let decrypted = if let Some(ref mut ch) = self.channel {
            ch.decrypt(&payload)?
        } else {
            payload
        };

        Ok(Frame::new(channel, decrypted))
    }

    async fn write_frame(&mut self, frame: Frame) -> Result<()> {
        let stream = self.stream.as_mut().ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        let payload = if let Some(ref mut ch) = self.channel {
            ch.encrypt(&frame.payload)?
        } else {
            frame.payload
        };

        let len = payload.len();
        let header = [
            frame.channel as u8,
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
        ];

        stream.write_all(&header).await?;
        stream.write_all(&payload).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Enable/disable black screen on remote
    pub async fn set_black_screen(&mut self, enabled: bool) -> Result<()> {
        let cmd = if enabled {
            protocol::privacy::BLACK_SCREEN_ON
        } else {
            protocol::privacy::BLACK_SCREEN_OFF
        };
        self.write_frame(Frame::privacy(cmd)).await
    }

    /// Enable/disable input blocking on remote
    pub async fn set_input_block(&mut self, enabled: bool) -> Result<()> {
        let cmd = if enabled {
            protocol::privacy::INPUT_BLOCK_ON
        } else {
            protocol::privacy::INPUT_BLOCK_OFF
        };
        self.write_frame(Frame::privacy(cmd)).await
    }

    /// Send mouse event to remote
    pub async fn send_mouse(
        &mut self,
        x: i32,
        y: i32,
        event_type: &str,
        button: Option<u8>,
    ) -> Result<()> {
        let mut payload = Vec::new();

        match event_type {
            "move" => {
                payload.push(protocol::input::MOUSE_MOVE);
                payload.extend(&x.to_le_bytes());
                payload.extend(&y.to_le_bytes());
            }
            "down" | "up" => {
                payload.push(protocol::input::MOUSE_BUTTON);
                payload.push(button.unwrap_or(0));
                payload.push(if event_type == "down" { 1 } else { 0 });
                payload.extend(&x.to_le_bytes());
                payload.extend(&y.to_le_bytes());
            }
            "scroll" => {
                payload.push(protocol::input::MOUSE_SCROLL);
                payload.extend(&x.to_le_bytes()); // delta_x
                payload.extend(&y.to_le_bytes()); // delta_y
            }
            _ => return Ok(()),
        }

        self.write_frame(Frame::input(payload)).await
    }

    /// Send keyboard event to remote
    pub async fn send_key(&mut self, key_code: u16, pressed: bool) -> Result<()> {
        let mut payload = vec![
            if pressed {
                protocol::input::KEY_DOWN
            } else {
                protocol::input::KEY_UP
            }
        ];
        payload.extend(&key_code.to_le_bytes());
        payload.push(0); // Modifiers

        self.write_frame(Frame::input(payload)).await
    }

    /// Send client viewport resolution to host for adaptive scaling
    pub async fn send_resolution(&mut self, width: u16, height: u16) -> Result<()> {
        let mut payload = Vec::new();
        payload.extend(&width.to_le_bytes());
        payload.extend(&height.to_le_bytes());
        self.write_frame(Frame::control(protocol::control::RESOLUTION, &payload)).await
    }

    /// Request video frame
    pub async fn request_frame(&mut self) -> Result<()> {
        self.write_frame(Frame::new(Channel::Video, vec![0x03])).await
    }

    /// Request and receive a video frame from remote
    /// Returns (width, height, jpeg_data) or None if no frame available
    pub async fn request_and_receive_frame(&mut self) -> Result<Option<(u16, u16, Vec<u8>)>> {
        // Send frame request
        self.write_frame(Frame::new(Channel::Video, vec![0x03])).await?;

        // Read response frame
        let frame = self.read_frame().await?;

        if frame.channel != Channel::Video {
            // Not a video frame, might be control message
            return Ok(None);
        }

        // Video frame format:
        // [keyframe (1 byte)][width (2 bytes LE)][height (2 bytes LE)][timestamp (8 bytes)][data...]
        if frame.payload.len() < 13 {
            return Ok(None);
        }

        let width = u16::from_le_bytes([frame.payload[1], frame.payload[2]]);
        let height = u16::from_le_bytes([frame.payload[3], frame.payload[4]]);
        // Skip timestamp (bytes 5-12)
        let data = frame.payload[13..].to_vec();

        Ok(Some((width, height, data)))
    }

    /// Disconnect session
    pub async fn disconnect(mut self) -> Result<()> {
        self.write_frame(Frame::control(protocol::control::SESSION_END, &[])).await?;
        if let Some(mut stream) = self.stream.take() {
            let _ = stream.shutdown().await;
        }
        Ok(())
    }
}
