//! Host mode - allow others to connect to this PC

#![allow(dead_code)]

use anyhow::Result;
use tauri::Emitter;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use parking_lot::Mutex as SyncMutex;

use crate::capture::ScreenCapture;
use crate::crypto::{Identity, SecureChannel};
use crate::input::InputInjector;
use crate::p2p::{gather_p2p_info, choose_p2p_port, create_p2p_listener, accept_p2p_connection};
use crate::privacy::PrivacyMode;
use crate::protocol::{self, Channel, Frame};
use crate::transport::{ConnectionType, P2PInfo};

/// Callback type for connection request notifications
pub type ConnectionCallback = Box<dyn Fn(String) + Send + Sync>;

/// Pending connection awaiting user approval
pub struct PendingConnection {
    pub remote_id: String,
    pub response_tx: mpsc::Sender<bool>,
}

/// Host session - running on the PC being controlled
pub struct HostSession {
    identity: Identity,
    stream: Option<tokio_rustls::client::TlsStream<TcpStream>>,
    p2p_stream: Option<TcpStream>,
    channel: Option<SecureChannel>,
    capture: ScreenCapture,
    input: InputInjector,
    privacy: PrivacyMode,
    running: bool,
    pending_connection: Arc<SyncMutex<Option<PendingConnection>>>,
    connection_type: ConnectionType,
    p2p_enabled: bool,
    /// Target resolution from client (for adaptive scaling)
    target_resolution: Option<(u16, u16)>,
}

impl HostSession {
    /// Start hosting and connect to relay (with P2P enabled by default)
    pub async fn start(relay_address: String, identity: Identity) -> Result<Self> {
        Self::start_with_p2p(relay_address, identity, true).await
    }

    /// Start hosting with explicit P2P control
    pub async fn start_with_p2p(relay_address: String, identity: Identity, p2p_enabled: bool) -> Result<Self> {
        println!("[HOST] Starting host session, connecting to relay: {}", relay_address);
        println!("[HOST] P2P enabled: {}", p2p_enabled);

        // Parse address
        let (host, port) = relay_address
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid relay address"))?;
        let port: u16 = port.parse()?;
        println!("[HOST] Parsed address: host={}, port={}", host, port);

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

        // Register as endpoint with our ID
        let id = identity.device_id_raw();
        println!("[HOST] Registering as endpoint with ID: {}", id);
        stream.write_u8(0x01).await?; // Endpoint type
        // Use big-endian for protocol compatibility with Go server
        stream.write_all(&(id.len() as u16).to_be_bytes()).await?;
        stream.write_all(id.as_bytes()).await?;
        stream.flush().await?;
        println!("[HOST] Registration sent, host session initialized");

        // Initialize capture/input
        let capture = ScreenCapture::new()?;
        let input = InputInjector::new();
        let privacy = PrivacyMode::new();

        Ok(Self {
            identity,
            stream: Some(stream),
            p2p_stream: None,
            channel: None,
            capture,
            input,
            privacy,
            running: true,
            pending_connection: Arc::new(SyncMutex::new(None)),
            connection_type: ConnectionType::Relay,
            p2p_enabled,
            target_resolution: None,
        })
    }

    /// Get the current connection type
    pub fn connection_type(&self) -> ConnectionType {
        self.connection_type
    }

    /// Set P2P enabled state
    pub fn set_p2p_enabled(&mut self, enabled: bool) {
        self.p2p_enabled = enabled;
    }

    /// Get a reference to pending connection for external access
    pub fn pending_connection(&self) -> Arc<SyncMutex<Option<PendingConnection>>> {
        self.pending_connection.clone()
    }

    /// Main loop - handle incoming requests
    pub async fn run(&mut self) -> Result<()> {
        while self.running {
            self.run_once().await?;
        }
        Ok(())
    }

    /// Run one iteration - process a single frame
    pub async fn run_once(&mut self) -> Result<()> {
        self.run_once_internal(None::<&tauri::AppHandle>).await
    }

    /// Run one iteration with event emission support
    pub async fn run_once_with_events(&mut self, app_handle: &tauri::AppHandle) -> Result<()> {
        self.run_once_internal(Some(app_handle)).await
    }

    async fn run_once_internal<R: tauri::Runtime>(&mut self, app_handle: Option<&tauri::AppHandle<R>>) -> Result<()> {
        if !self.running {
            anyhow::bail!("Session stopped");
        }

        println!("[HOST] Waiting for frame...");
        let frame = self.read_frame().await?;
        println!("[HOST] Received frame on channel {:?}, payload len: {}", frame.channel, frame.payload.len());
        if !frame.payload.is_empty() {
            println!("[HOST] First payload byte: 0x{:02x}", frame.payload[0]);
        }

        match frame.channel {
            Channel::Control => {
                println!("[HOST] Handling control message");
                self.handle_control_with_events(&frame, app_handle).await?;
            }
            Channel::Input => {
                println!("[HOST] Handling input");
                self.handle_input(&frame).await?;
            }
            Channel::Privacy => {
                println!("[HOST] Handling privacy");
                self.handle_privacy(&frame).await?;
            }
            Channel::Video => {
                println!("[HOST] Video request - sending frame");
                self.send_video_frame().await?;
            }
            Channel::Clipboard => {
                println!("[HOST] Handling clipboard");
                self.handle_clipboard_with_events(&frame, app_handle).await?;
            }
            _ => {
                println!("[HOST] Unknown channel");
            }
        }
        Ok(())
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

        // Decrypt if channel established
        let decrypted = if let Some(ref mut ch) = self.channel {
            ch.decrypt(&payload)?
        } else {
            payload
        };

        Ok(Frame::new(channel, decrypted))
    }

    async fn write_frame(&mut self, frame: Frame) -> Result<()> {
        let stream = self.stream.as_mut().ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        // Encrypt if channel established
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

    async fn handle_control(&mut self, frame: &Frame) -> Result<()> {
        self.handle_control_with_events::<tauri::Wry>(frame, None).await
    }

    async fn handle_control_with_events<R: tauri::Runtime>(&mut self, frame: &Frame, app_handle: Option<&tauri::AppHandle<R>>) -> Result<()> {
        if frame.payload.is_empty() {
            println!("[HOST] Control frame has empty payload");
            return Ok(());
        }

        println!("[HOST] Control message type: 0x{:02x}", frame.payload[0]);

        match frame.payload[0] {
            protocol::control::HANDSHAKE => {
                println!("[HOST] Received HANDSHAKE");
                // Noise handshake from client
                let mut responder = self.identity.create_responder()?;
                let mut buf = vec![0u8; 65535];

                // Read message
                responder.read_message(&frame.payload[1..], &mut buf)?;

                // Send response
                let len = responder.write_message(&[], &mut buf)?;
                let mut response = vec![protocol::control::HANDSHAKE];
                response.extend_from_slice(&buf[..len]);
                self.write_frame(Frame::new(Channel::Control, response)).await?;

                // Complete handshake
                if responder.is_handshake_finished() {
                    self.channel = Some(SecureChannel::from_handshake(responder)?);
                }
            }
            protocol::control::SESSION_REQUEST => {
                // Extract remote ID from payload (bytes after the message type)
                let remote_id = if frame.payload.len() > 1 {
                    String::from_utf8_lossy(&frame.payload[1..]).to_string()
                } else {
                    "Unknown".to_string()
                };

                println!("[HOST] Received SESSION_REQUEST from: {}", remote_id);

                // Create channel for user response
                let (tx, mut rx) = mpsc::channel::<bool>(1);

                // Store pending connection
                {
                    let mut pending = self.pending_connection.lock();
                    *pending = Some(PendingConnection {
                        remote_id: remote_id.clone(),
                        response_tx: tx,
                    });
                }

                // Emit event to frontend to show approval dialog
                if let Some(handle) = app_handle {
                    let _ = handle.emit("connection-request", serde_json::json!({
                        "remote_id": remote_id.clone()
                    }));
                    println!("[HOST] Emitted connection-request event for: {}", remote_id);
                }

                // Wait for user response (with timeout)
                let accepted = tokio::time::timeout(
                    tokio::time::Duration::from_secs(30),
                    rx.recv()
                ).await.unwrap_or(None).unwrap_or(false);

                // Clear pending connection
                {
                    let mut pending = self.pending_connection.lock();
                    *pending = None;
                }

                if accepted {
                    // User accepted - send SESSION_ACCEPT
                    self.write_frame(Frame::control(protocol::control::SESSION_ACCEPT, &[0x01])).await?;
                    println!("[HOST] User accepted - sent SESSION_ACCEPT");

                    // Emit connected event
                    if let Some(handle) = app_handle {
                        let _ = handle.emit("connection-accepted", serde_json::json!({
                            "remote_id": remote_id
                        }));
                    }
                } else {
                    // User declined or timeout - send SESSION_REJECT
                    self.write_frame(Frame::control(protocol::control::SESSION_END, &[0x00])).await?;
                    println!("[HOST] User declined - sent SESSION_END");
                }
            }
            protocol::control::SESSION_END => {
                self.running = false;
                self.privacy.disable_all()?;

                // Emit disconnected event
                if let Some(handle) = app_handle {
                    let _ = handle.emit("connection-ended", serde_json::json!({}));
                }
            }
            protocol::control::KEEPALIVE => {
                self.write_frame(Frame::control(protocol::control::KEEPALIVE, &[])).await?;
            }
            protocol::control::P2P_OFFER => {
                println!("[HOST] Received P2P_OFFER");
                // Parse remote P2P info
                if let Ok(remote_info) = P2PInfo::decode(&frame.payload[1..]) {
                    println!("[HOST] Remote P2P info: {:?}", remote_info);

                    // Gather our P2P info
                    let my_id = self.identity.device_id_raw();
                    let p2p_port = choose_p2p_port(&my_id);
                    let local_info = gather_p2p_info(self.p2p_enabled, p2p_port).await;

                    // Send P2P answer
                    let answer_data = local_info.encode();
                    self.write_frame(Frame::control(protocol::control::P2P_ANSWER, &answer_data)).await?;
                    println!("[HOST] Sent P2P_ANSWER");

                    // If either side has P2P enabled, prepare for P2P connection
                    if remote_info.p2p_enabled || local_info.p2p_enabled {
                        // Start P2P listener
                        if let Ok(listener) = create_p2p_listener(p2p_port).await {
                            // Wait for P2P connection or P2P_FAILED message
                            tokio::select! {
                                p2p_result = accept_p2p_connection(&listener, remote_info.public_addr) => {
                                    if let Ok(Some(transport)) = p2p_result {
                                        println!("[HOST] P2P connection accepted!");
                                        self.p2p_stream = Some(transport.stream);
                                        self.connection_type = ConnectionType::P2P;

                                        // Emit connection type change event
                                        if let Some(handle) = app_handle {
                                            let _ = handle.emit("connection-type-changed", serde_json::json!({
                                                "type": "P2P"
                                            }));
                                        }
                                    }
                                }
                                // Also check for relay messages (P2P_FAILED)
                                relay_frame = self.read_frame() => {
                                    if let Ok(f) = relay_frame {
                                        if f.channel == Channel::Control
                                            && !f.payload.is_empty()
                                            && f.payload[0] == protocol::control::P2P_FAILED
                                        {
                                            println!("[HOST] Client reported P2P failed, staying on relay");
                                        } else if f.payload[0] == protocol::control::P2P_READY {
                                            println!("[HOST] Client reported P2P ready");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            protocol::control::P2P_READY => {
                println!("[HOST] Received P2P_READY - P2P connection confirmed");
            }
            protocol::control::P2P_FAILED => {
                println!("[HOST] Received P2P_FAILED - using relay");
                self.connection_type = ConnectionType::Relay;
            }
            protocol::control::RESOLUTION => {
                // Client sends target viewport resolution
                if frame.payload.len() >= 5 {
                    let width = u16::from_le_bytes([frame.payload[1], frame.payload[2]]);
                    let height = u16::from_le_bytes([frame.payload[3], frame.payload[4]]);
                    println!("[HOST] Client resolution: {}x{}", width, height);
                    self.target_resolution = Some((width, height));
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_input(&mut self, frame: &Frame) -> Result<()> {
        if frame.payload.is_empty() {
            return Ok(());
        }

        match frame.payload[0] {
            protocol::input::MOUSE_MOVE => {
                if frame.payload.len() >= 9 {
                    let x = i32::from_le_bytes(frame.payload[1..5].try_into()?);
                    let y = i32::from_le_bytes(frame.payload[5..9].try_into()?);
                    self.input.move_mouse(x, y)?;
                }
            }
            protocol::input::MOUSE_BUTTON => {
                if frame.payload.len() >= 11 {
                    let button = frame.payload[1];
                    let pressed = frame.payload[2] != 0;
                    let x = i32::from_le_bytes(frame.payload[3..7].try_into()?);
                    let y = i32::from_le_bytes(frame.payload[7..11].try_into()?);
                    self.input.mouse_button(button, pressed, x, y)?;
                }
            }
            protocol::input::MOUSE_SCROLL => {
                if frame.payload.len() >= 9 {
                    let dx = i32::from_le_bytes(frame.payload[1..5].try_into()?);
                    let dy = i32::from_le_bytes(frame.payload[5..9].try_into()?);
                    self.input.mouse_scroll(dx, dy)?;
                }
            }
            protocol::input::KEY_DOWN | protocol::input::KEY_UP => {
                if frame.payload.len() >= 4 {
                    let key = u16::from_le_bytes(frame.payload[1..3].try_into()?);
                    let pressed = frame.payload[0] == protocol::input::KEY_DOWN;
                    self.input.key_event(key, pressed)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_privacy(&mut self, frame: &Frame) -> Result<()> {
        if frame.payload.is_empty() {
            return Ok(());
        }

        match frame.payload[0] {
            protocol::privacy::BLACK_SCREEN_ON => {
                self.privacy.enable_black_screen()?;
            }
            protocol::privacy::BLACK_SCREEN_OFF => {
                self.privacy.disable_black_screen()?;
            }
            protocol::privacy::INPUT_BLOCK_ON => {
                self.privacy.block_input()?;
            }
            protocol::privacy::INPUT_BLOCK_OFF => {
                self.privacy.unblock_input()?;
            }
            _ => {}
        }

        // Send acknowledgment
        let status = vec![
            protocol::privacy::STATUS_ACK,
            self.privacy.is_black_screen_active() as u8,
            self.privacy.is_input_blocked() as u8,
        ];
        self.write_frame(Frame::new(Channel::Privacy, status)).await?;
        Ok(())
    }

    async fn send_video_frame(&mut self) -> Result<()> {
        let (width, height, data) = self.capture.capture()?;

        let mut payload = Vec::with_capacity(13 + data.len());
        payload.push(0x01); // Keyframe
        payload.extend(&(width as u16).to_le_bytes());
        payload.extend(&(height as u16).to_le_bytes());
        payload.extend(&0u64.to_le_bytes()); // Timestamp
        payload.extend(&data);

        self.write_frame(Frame::video(payload)).await
    }

    async fn handle_clipboard_with_events<R: tauri::Runtime>(
        &mut self,
        frame: &Frame,
        app_handle: Option<&tauri::AppHandle<R>>,
    ) -> Result<()> {
        use crate::clipboard::{ClipboardManager, ClipboardData};

        if frame.payload.is_empty() {
            return Ok(());
        }

        match frame.payload[0] {
            protocol::clipboard::CLIPBOARD_REQUEST => {
                println!("[HOST] Remote requested clipboard");
                // Get local clipboard and send it
                let clipboard = ClipboardManager::new();
                if let Ok(Some(data)) = clipboard.get_clipboard() {
                    let encoded = data.encode();
                    self.write_frame(Frame::clipboard(protocol::clipboard::CLIPBOARD_DATA, &encoded)).await?;
                    println!("[HOST] Sent clipboard data ({} bytes)", encoded.len());
                }
            }
            protocol::clipboard::CLIPBOARD_DATA => {
                println!("[HOST] Received clipboard data from remote");
                // Decode and set local clipboard
                if frame.payload.len() > 1 {
                    if let Ok(data) = ClipboardData::decode(&frame.payload[1..]) {
                        let clipboard = ClipboardManager::new();
                        clipboard.update_hash(&data);
                        if let Err(e) = clipboard.set_clipboard(&data) {
                            eprintln!("[HOST] Failed to set clipboard: {}", e);
                        } else {
                            println!("[HOST] Clipboard updated from remote");
                            // Notify frontend
                            if let Some(handle) = app_handle {
                                let _ = handle.emit("clipboard-received", serde_json::json!({
                                    "type": match &data {
                                        ClipboardData::Text(_) => "text",
                                        ClipboardData::Image { .. } => "image",
                                        ClipboardData::Files(_) => "files",
                                    }
                                }));
                            }
                        }
                    }
                }
            }
            protocol::clipboard::CLIPBOARD_CHANGED => {
                println!("[HOST] Remote clipboard changed notification");
                // Optionally auto-fetch the clipboard
            }
            _ => {}
        }
        Ok(())
    }

    /// Stop hosting
    pub async fn stop(mut self) -> Result<()> {
        self.running = false;
        self.privacy.disable_all()?;
        if let Some(mut stream) = self.stream.take() {
            let _ = stream.shutdown().await;
        }
        Ok(())
    }
}
