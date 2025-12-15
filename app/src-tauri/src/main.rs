//! SecureDesk - Unified Remote Desktop Application
//!
//! Single executable that works as BOTH:
//! - Host: Allow others to connect to this PC
//! - Client: Connect to other PCs
//!
//! Like AnyDesk/TeamViewer - one app does everything.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod crypto;
mod host;
mod client;
mod capture;
mod input;
mod privacy;
mod protocol;
mod qos;
mod config;
mod transport;
mod stun;
mod p2p;
mod license;
mod clipboard;
mod recording;
mod cli;
mod sso;

use parking_lot::Mutex as SyncMutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    Manager, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
};
use tokio::sync::Mutex as AsyncMutex;

/// Relay server addresses (multiple for failover and load balancing)
const RELAY_SERVERS: &[&str] = &[
    "relay.securedesk.one:8443",
    "relay2.securedesk.one:8443",
];

/// Client session with metadata
pub struct ClientSessionEntry {
    session: client::ClientSession,
    remote_id: String,
    remote_name: String,
    connected_at: u64,
}

/// Global application state
struct AppState {
    identity: SyncMutex<crypto::Identity>,
    host_session: AsyncMutex<Option<host::HostSession>>,
    /// Multiple client sessions - key is session_id (auto-generated)
    client_sessions: AsyncMutex<HashMap<String, ClientSessionEntry>>,
    /// Currently active session ID for commands without explicit session_id
    active_session_id: SyncMutex<Option<String>>,
    /// Counter for generating session IDs
    session_counter: AtomicU64,
    relay_addresses: SyncMutex<Vec<String>>,
    connection_config: SyncMutex<config::ConnectionConfig>,
    license_manager: SyncMutex<license::LicenseManager>,
    clipboard_manager: clipboard::ClipboardManager,
    recording_manager: recording::RecordingManager,
    sso_manager: AsyncMutex<sso::SsoManager>,
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// Get this device's ID (for sharing)
#[tauri::command]
fn get_device_id(state: tauri::State<Arc<AppState>>) -> String {
    state.identity.lock().device_id()
}

/// Regenerate device ID (creates new identity)
#[tauri::command]
fn regenerate_device_id(state: tauri::State<Arc<AppState>>) -> Result<String, String> {
    let new_identity = crypto::Identity::regenerate()
        .map_err(|e| e.to_string())?;
    let new_id = new_identity.device_id();
    *state.identity.lock() = new_identity;
    Ok(new_id)
}

/// Set the relay server addresses
#[tauri::command]
fn set_relay_address(state: tauri::State<Arc<AppState>>, address: String) {
    // Support comma-separated list of addresses
    let addresses: Vec<String> = address
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    *state.relay_addresses.lock() = addresses;
}

/// Start listening for incoming connections (host mode)
/// Tries each relay server until one works
#[tauri::command]
async fn start_host_listener(
    state: tauri::State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let relays = state.relay_addresses.lock().clone();
    let identity = state.identity.lock().clone();

    let mut last_error = String::from("No relay servers configured");

    for relay in relays {
        println!("[MAIN] Trying to connect to relay: {}", relay);
        match host::HostSession::start(relay.clone(), identity.clone()).await {
            Ok(session) => {
                println!("[MAIN] Connected to relay: {}", relay);
                *state.host_session.lock().await = Some(session);

                // Spawn background task to run the host session
                let state_clone = state.inner().clone();
                let app_handle_clone = app_handle.clone();
                println!("[MAIN] Spawning background host session task");
                tokio::spawn(async move {
                    println!("[MAIN-TASK] Host session background task started");
                    loop {
                        // Take the session to run it
                        let mut session_opt = state_clone.host_session.lock().await;
                        if let Some(ref mut session) = *session_opt {
                            // Run one iteration of the host loop
                            match session.run_once_with_events(&app_handle_clone).await {
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("[MAIN-TASK] Host session error: {}", e);
                                    // On error, clear the session and try to reconnect
                                    *session_opt = None;
                                    drop(session_opt);

                                    // Try to reconnect after a delay
                                    println!("[MAIN-TASK] Reconnecting in 5 seconds...");
                                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                                    // Attempt reconnection
                                    let relays = state_clone.relay_addresses.lock().clone();
                                    let identity = state_clone.identity.lock().clone();
                                    for relay in relays {
                                        println!("[MAIN-TASK] Trying relay: {}", relay);
                                        if let Ok(new_session) = host::HostSession::start(relay, identity.clone()).await {
                                            println!("[MAIN-TASK] Reconnected successfully");
                                            *state_clone.host_session.lock().await = Some(new_session);
                                            break;
                                        }
                                    }
                                    continue;
                                }
                            }
                        } else {
                            drop(session_opt);
                            // No session, wait a bit
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        }
                    }
                });

                return Ok(());
            }
            Err(e) => {
                println!("[MAIN] Relay {} failed: {}", relay, e);
                last_error = format!("Relay {} failed: {}", relay, e);
                continue;
            }
        }
    }

    Err(last_error)
}

/// Session info for frontend display
#[derive(serde::Serialize, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub remote_id: String,
    pub remote_name: String,
    pub connected_at: u64,
    pub is_active: bool,
    pub connection_type: String,
}

/// Connect to a remote device (client mode)
/// Tries each relay server until one works
/// Returns the session_id for multi-session management
#[tauri::command]
async fn connect_to_remote(
    state: tauri::State<'_, Arc<AppState>>,
    remote_id: String,
    remote_name: Option<String>,
) -> Result<String, String> {
    let relays = state.relay_addresses.lock().clone();
    let identity = state.identity.lock().clone();

    let mut last_error = String::from("No relay servers configured");

    for relay in relays {
        match client::ClientSession::connect(relay.clone(), remote_id.clone(), identity.clone()).await {
            Ok(session) => {
                // Generate a unique session ID
                let counter = state.session_counter.fetch_add(1, Ordering::SeqCst);
                let session_id = format!("session_{}", counter);

                // Get current timestamp
                let connected_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let entry = ClientSessionEntry {
                    session,
                    remote_id: remote_id.clone(),
                    remote_name: remote_name.clone().unwrap_or_else(|| remote_id.clone()),
                    connected_at,
                };

                // Add to sessions map
                state.client_sessions.lock().await.insert(session_id.clone(), entry);

                // Set as active session
                *state.active_session_id.lock() = Some(session_id.clone());

                println!("[MAIN] Connected to {} as session {}", remote_id, session_id);
                return Ok(session_id);
            }
            Err(e) => {
                last_error = format!("Relay {} failed: {}", relay, e);
                continue;
            }
        }
    }

    Err(last_error)
}

/// Disconnect a session by ID, or the active session if no ID provided
#[tauri::command]
async fn disconnect_session(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.remove(&target_id) {
        println!("[MAIN] Disconnecting session {}", target_id);
        entry.session.disconnect().await.map_err(|e| e.to_string())?;

        // If this was the active session, set another one as active (or None)
        let mut active_id = state.active_session_id.lock();
        if active_id.as_ref() == Some(&target_id) {
            *active_id = sessions.keys().next().cloned();
        }
    }
    Ok(())
}

/// Disconnect all sessions
#[tauri::command]
async fn disconnect_all_sessions(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut sessions = state.client_sessions.lock().await;
    let session_ids: Vec<String> = sessions.keys().cloned().collect();

    for session_id in session_ids {
        if let Some(entry) = sessions.remove(&session_id) {
            println!("[MAIN] Disconnecting session {}", session_id);
            let _ = entry.session.disconnect().await;
        }
    }

    *state.active_session_id.lock() = None;
    Ok(())
}

/// List all active sessions
#[tauri::command]
async fn list_sessions(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<SessionInfo>, String> {
    let sessions = state.client_sessions.lock().await;
    let active_id = state.active_session_id.lock().clone();

    Ok(sessions
        .iter()
        .map(|(id, entry)| SessionInfo {
            session_id: id.clone(),
            remote_id: entry.remote_id.clone(),
            remote_name: entry.remote_name.clone(),
            connected_at: entry.connected_at,
            is_active: active_id.as_ref() == Some(id),
            connection_type: entry.session.connection_type().to_string(),
        })
        .collect())
}

/// Set the active session
#[tauri::command]
async fn set_active_session(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<(), String> {
    let sessions = state.client_sessions.lock().await;
    if sessions.contains_key(&session_id) {
        *state.active_session_id.lock() = Some(session_id);
        Ok(())
    } else {
        Err(format!("Session {} not found", session_id))
    }
}

/// Get the currently active session ID
#[tauri::command]
fn get_active_session(state: tauri::State<Arc<AppState>>) -> Option<String> {
    state.active_session_id.lock().clone()
}

/// Get session count
#[tauri::command]
async fn get_session_count(state: tauri::State<'_, Arc<AppState>>) -> Result<usize, String> {
    Ok(state.client_sessions.lock().await.len())
}

/// Toggle black screen on remote (when in client mode)
#[tauri::command]
async fn set_black_screen(
    state: tauri::State<'_, Arc<AppState>>,
    enabled: bool,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.set_black_screen(enabled).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Toggle input blocking on remote (when in client mode)
#[tauri::command]
async fn set_input_block(
    state: tauri::State<'_, Arc<AppState>>,
    enabled: bool,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.set_input_block(enabled).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Send mouse event to remote
#[tauri::command]
async fn send_mouse(
    state: tauri::State<'_, Arc<AppState>>,
    x: i32,
    y: i32,
    event_type: String,
    button: Option<u8>,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.send_mouse(x, y, &event_type, button).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Send key event to remote
#[tauri::command]
async fn send_key(
    state: tauri::State<'_, Arc<AppState>>,
    key_code: u16,
    pressed: bool,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.send_key(key_code, pressed).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Send client viewport resolution to host for adaptive scaling
#[tauri::command]
async fn send_resolution(
    state: tauri::State<'_, Arc<AppState>>,
    width: u16,
    height: u16,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.send_resolution(width, height).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Video frame data returned to frontend
#[derive(serde::Serialize)]
struct VideoFrame {
    width: u16,
    height: u16,
    data: String, // Base64 encoded JPEG
}

/// Request and receive a video frame from remote
#[tauri::command]
async fn request_video_frame(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: Option<String>,
) -> Result<Option<VideoFrame>, String> {
    let target_id = match session_id.or_else(|| state.active_session_id.lock().clone()) {
        Some(id) => id,
        None => return Ok(None),
    };

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        match entry.session.request_and_receive_frame().await {
            Ok(Some((width, height, data))) => {
                // Write frame to recording if recording is active
                if let Err(e) = state.recording_manager.write_frame(width, height, &data) {
                    // Log but don't fail the frame request
                    eprintln!("[RECORDING] Failed to write frame: {}", e);
                }

                // Encode frame data as base64 for transfer to frontend
                use base64::{Engine as _, engine::general_purpose::STANDARD};
                let encoded = STANDARD.encode(&data);
                Ok(Some(VideoFrame { width, height, data: encoded }))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    } else {
        Ok(None)
    }
}

/// Respond to pending connection request (accept or decline)
#[tauri::command]
async fn respond_to_connection(
    state: tauri::State<'_, Arc<AppState>>,
    accept: bool,
) -> Result<(), String> {
    let session_opt = state.host_session.lock().await;
    if let Some(ref session) = *session_opt {
        let pending = session.pending_connection();
        let mut pending_lock = pending.lock();
        if let Some(pending_conn) = pending_lock.take() {
            // Send response to the waiting host session
            let _ = pending_conn.response_tx.try_send(accept);
            println!("[MAIN] Sent connection response: accept={}", accept);
            Ok(())
        } else {
            Err("No pending connection".to_string())
        }
    } else {
        Err("No host session active".to_string())
    }
}

// ============================================================================
// P2P Commands
// ============================================================================

/// Get P2P enabled state
#[tauri::command]
fn get_p2p_enabled(state: tauri::State<Arc<AppState>>) -> bool {
    state.connection_config.lock().p2p_enabled
}

/// Set P2P enabled state
#[tauri::command]
fn set_p2p_enabled(state: tauri::State<Arc<AppState>>, enabled: bool) -> Result<(), String> {
    let mut config = state.connection_config.lock();
    config.set_p2p_enabled(enabled).map_err(|e| e.to_string())?;
    Ok(())
}

/// Get current connection type (for active session or specified session)
#[tauri::command]
async fn get_connection_type(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: Option<String>,
) -> Result<String, String> {
    // Check specified or active client session first
    let target_id = session_id.or_else(|| state.active_session_id.lock().clone());

    if let Some(id) = target_id {
        let sessions = state.client_sessions.lock().await;
        if let Some(entry) = sessions.get(&id) {
            return Ok(entry.session.connection_type().to_string());
        }
    }

    // Check host session
    let host_opt = state.host_session.lock().await;
    if let Some(ref session) = *host_opt {
        return Ok(session.connection_type().to_string());
    }

    Ok("None".to_string())
}

// ============================================================================
// Trusted Device Commands
// ============================================================================

/// Check if device is trusted
#[tauri::command]
fn is_device_trusted(state: tauri::State<Arc<AppState>>, device_id: String) -> bool {
    state.connection_config.lock().is_trusted(&device_id)
}

/// Add trusted device
#[tauri::command]
fn add_trusted_device(
    state: tauri::State<Arc<AppState>>,
    device_id: String,
    name: Option<String>,
) -> Result<(), String> {
    let mut config = state.connection_config.lock();
    config.add_trusted_device(&device_id, name).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove trusted device
#[tauri::command]
fn remove_trusted_device(
    state: tauri::State<Arc<AppState>>,
    device_id: String,
) -> Result<(), String> {
    let mut config = state.connection_config.lock();
    config.remove_trusted_device(&device_id).map_err(|e| e.to_string())?;
    Ok(())
}

/// Trusted device info for frontend
#[derive(serde::Serialize)]
struct TrustedDeviceInfo {
    device_id: String,
    name: Option<String>,
    trusted_at: u64,
    last_connected: Option<u64>,
}

/// Get list of trusted devices
#[tauri::command]
fn get_trusted_devices(state: tauri::State<Arc<AppState>>) -> Vec<TrustedDeviceInfo> {
    let config = state.connection_config.lock();
    config
        .get_trusted_devices()
        .iter()
        .map(|d| TrustedDeviceInfo {
            device_id: d.device_id.clone(),
            name: d.name.clone(),
            trusted_at: d.trusted_at,
            last_connected: d.last_connected,
        })
        .collect()
}

// ============================================================================
// Settings Commands
// ============================================================================

/// Settings info for frontend
#[derive(serde::Serialize)]
struct SettingsInfo {
    start_with_windows: bool,
    minimize_to_tray: bool,
    show_notifications: bool,
    p2p_enabled: bool,
    connection_quality: String,
    require_approval: bool,
    lock_on_disconnect: bool,
    session_timeout: u32,
    hide_from_address_book: bool,
}

/// Get all settings
#[tauri::command]
fn get_settings(state: tauri::State<Arc<AppState>>) -> SettingsInfo {
    let config = state.connection_config.lock();
    let settings = config.get_settings();
    SettingsInfo {
        start_with_windows: settings.start_with_windows,
        minimize_to_tray: settings.minimize_to_tray,
        show_notifications: settings.show_notifications,
        p2p_enabled: settings.p2p_enabled,
        connection_quality: settings.connection_quality.clone(),
        require_approval: settings.require_approval,
        lock_on_disconnect: settings.lock_on_disconnect,
        session_timeout: settings.session_timeout,
        hide_from_address_book: settings.hide_from_address_book,
    }
}

/// Update a boolean setting
#[tauri::command]
fn set_setting_bool(
    state: tauri::State<Arc<AppState>>,
    key: String,
    value: bool,
) -> Result<(), String> {
    let mut config = state.connection_config.lock();
    config.update_setting(&key, config::SettingValue::Bool(value))
        .map_err(|e| e.to_string())
}

/// Update a string setting
#[tauri::command]
fn set_setting_string(
    state: tauri::State<Arc<AppState>>,
    key: String,
    value: String,
) -> Result<(), String> {
    let mut config = state.connection_config.lock();
    config.update_setting(&key, config::SettingValue::String(value))
        .map_err(|e| e.to_string())
}

/// Update a number setting
#[tauri::command]
fn set_setting_number(
    state: tauri::State<Arc<AppState>>,
    key: String,
    value: u32,
) -> Result<(), String> {
    let mut config = state.connection_config.lock();
    config.update_setting(&key, config::SettingValue::Number(value))
        .map_err(|e| e.to_string())
}

// ============================================================================
// Clipboard Commands
// ============================================================================

/// Clipboard content info for frontend
#[derive(serde::Serialize, serde::Deserialize)]
struct ClipboardContent {
    data_type: String, // "text", "image", "files"
    text: Option<String>,
    image_data: Option<String>, // Base64 encoded
    files: Option<Vec<String>>,
}

/// Get local clipboard content
#[tauri::command]
fn get_local_clipboard(state: tauri::State<Arc<AppState>>) -> Result<Option<ClipboardContent>, String> {
    match state.clipboard_manager.get_clipboard() {
        Ok(Some(data)) => {
            let content = match data {
                clipboard::ClipboardData::Text(text) => ClipboardContent {
                    data_type: "text".to_string(),
                    text: Some(text),
                    image_data: None,
                    files: None,
                },
                clipboard::ClipboardData::Image { data, .. } => {
                    use base64::{Engine as _, engine::general_purpose::STANDARD};
                    ClipboardContent {
                        data_type: "image".to_string(),
                        text: None,
                        image_data: Some(STANDARD.encode(&data)),
                        files: None,
                    }
                }
                clipboard::ClipboardData::Files(files) => ClipboardContent {
                    data_type: "files".to_string(),
                    text: None,
                    image_data: None,
                    files: Some(files),
                },
            };
            Ok(Some(content))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Set local clipboard content
#[tauri::command]
fn set_local_clipboard(
    state: tauri::State<Arc<AppState>>,
    content: ClipboardContent,
) -> Result<(), String> {
    let data = match content.data_type.as_str() {
        "text" => {
            let text = content.text.ok_or("Missing text content")?;
            clipboard::ClipboardData::Text(text)
        }
        "image" => {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            let encoded = content.image_data.ok_or("Missing image data")?;
            let data = STANDARD.decode(&encoded).map_err(|e| e.to_string())?;
            clipboard::ClipboardData::Image { width: 0, height: 0, data }
        }
        "files" => {
            let files = content.files.ok_or("Missing files")?;
            clipboard::ClipboardData::Files(files)
        }
        _ => return Err("Unknown clipboard data type".to_string()),
    };

    state.clipboard_manager.update_hash(&data);
    state.clipboard_manager.set_clipboard(&data).map_err(|e| e.to_string())
}

/// Send clipboard to remote device
#[tauri::command]
async fn send_clipboard_to_remote(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    // Get local clipboard
    let data = match state.clipboard_manager.get_clipboard() {
        Ok(Some(d)) => d,
        Ok(None) => return Err("Clipboard is empty".to_string()),
        Err(e) => return Err(e.to_string()),
    };

    let encoded = data.encode();

    // Send via client session
    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.send_clipboard(&encoded).await.map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

/// Request clipboard from remote device
#[tauri::command]
async fn request_remote_clipboard(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: Option<String>,
) -> Result<(), String> {
    let target_id = session_id
        .or_else(|| state.active_session_id.lock().clone())
        .ok_or("No active session")?;

    let mut sessions = state.client_sessions.lock().await;
    if let Some(entry) = sessions.get_mut(&target_id) {
        entry.session.request_clipboard().await.map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

/// Get clipboard sync enabled state
#[tauri::command]
fn get_clipboard_sync_enabled(state: tauri::State<Arc<AppState>>) -> bool {
    state.clipboard_manager.is_sync_enabled()
}

/// Set clipboard sync enabled state
#[tauri::command]
fn set_clipboard_sync_enabled(state: tauri::State<Arc<AppState>>, enabled: bool) {
    state.clipboard_manager.set_sync_enabled(enabled);
}

// ============================================================================
// Recording Commands
// ============================================================================

/// Start recording the session
#[tauri::command]
fn start_recording(
    state: tauri::State<Arc<AppState>>,
    remote_device_id: String,
    remote_device_name: String,
) -> Result<(), String> {
    state.recording_manager
        .start_recording(&remote_device_id, &remote_device_name)
        .map_err(|e| e.to_string())
}

/// Stop recording the session
#[tauri::command]
fn stop_recording(state: tauri::State<Arc<AppState>>) -> Result<String, String> {
    state.recording_manager
        .stop_recording()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

/// Check if currently recording
#[tauri::command]
fn is_recording(state: tauri::State<Arc<AppState>>) -> bool {
    state.recording_manager.is_recording()
}

/// Get recording status
#[tauri::command]
fn get_recording_status(state: tauri::State<Arc<AppState>>) -> Option<recording::RecordingStatus> {
    state.recording_manager.status()
}

/// List all recordings
#[tauri::command]
fn list_recordings() -> Result<Vec<recording::RecordingInfo>, String> {
    recording::list_recordings().map_err(|e| e.to_string())
}

/// Delete a recording
#[tauri::command]
fn delete_recording(path: String) -> Result<(), String> {
    recording::delete_recording(&path).map_err(|e| e.to_string())
}

/// Open recordings folder
#[tauri::command]
fn open_recordings_folder() -> Result<(), String> {
    let dir = recording::SessionRecorder::recordings_directory()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ============================================================================
// SSO/OIDC Commands
// ============================================================================

/// Get SSO status and info
#[tauri::command]
async fn get_sso_info(state: tauri::State<'_, Arc<AppState>>) -> Result<sso::SsoInfo, String> {
    let manager = state.sso_manager.lock().await;
    Ok(sso::SsoInfo::from_manager(&manager))
}

/// List configured SSO providers
#[tauri::command]
async fn list_sso_providers(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    let manager = state.sso_manager.lock().await;
    Ok(manager.list_providers().iter().map(|p| p.name.clone()).collect())
}

/// SSO provider config for frontend
#[derive(serde::Serialize, serde::Deserialize)]
struct SsoProviderConfig {
    name: String,
    provider_type: String, // "azure", "okta", "google", "custom"
    client_id: String,
    client_secret: Option<String>,
    tenant_id: Option<String>,  // For Azure
    domain: Option<String>,      // For Okta
    discovery_url: Option<String>, // For custom OIDC
}

/// Add a new SSO provider
#[tauri::command]
async fn add_sso_provider(
    state: tauri::State<'_, Arc<AppState>>,
    config: SsoProviderConfig,
) -> Result<(), String> {
    let provider = match config.provider_type.as_str() {
        "azure" => {
            let tenant_id = config.tenant_id.ok_or("Missing tenant_id for Azure AD")?;
            sso::OidcProvider::azure_ad(&tenant_id, &config.client_id)
        }
        "okta" => {
            let domain = config.domain.ok_or("Missing domain for Okta")?;
            sso::OidcProvider::okta(&domain, &config.client_id)
        }
        "google" => {
            let client_secret = config.client_secret.ok_or("Missing client_secret for Google")?;
            sso::OidcProvider::google(&config.client_id, &client_secret)
        }
        "custom" => {
            let discovery_url = config.discovery_url.ok_or("Missing discovery_url for custom OIDC")?;
            sso::OidcProvider::from_discovery(&discovery_url, &config.client_id)
                .await
                .map_err(|e| e.to_string())?
        }
        _ => return Err(format!("Unknown provider type: {}", config.provider_type)),
    };

    let mut manager = state.sso_manager.lock().await;
    manager.add_provider(provider).map_err(|e| e.to_string())
}

/// Remove an SSO provider
#[tauri::command]
async fn remove_sso_provider(
    state: tauri::State<'_, Arc<AppState>>,
    name: String,
) -> Result<(), String> {
    let mut manager = state.sso_manager.lock().await;
    manager.remove_provider(&name).map_err(|e| e.to_string())
}

/// SSO login response
#[derive(serde::Serialize)]
struct SsoLoginResponse {
    auth_url: String,
    redirect_uri: String,
}

/// Start SSO login flow - returns URL to open in browser
#[tauri::command]
async fn start_sso_login(
    state: tauri::State<'_, Arc<AppState>>,
    provider_name: String,
) -> Result<SsoLoginResponse, String> {
    let manager = state.sso_manager.lock().await;
    let provider = manager
        .config()
        .get_provider(&provider_name)
        .ok_or(format!("Provider {} not found", provider_name))?
        .clone();

    let (auth_url, redirect_uri, _pkce) = manager
        .start_login(&provider)
        .map_err(|e| e.to_string())?;

    Ok(SsoLoginResponse { auth_url, redirect_uri })
}

/// Complete SSO login - waits for callback and exchanges code for tokens
#[tauri::command]
async fn complete_sso_login(
    state: tauri::State<'_, Arc<AppState>>,
    provider_name: String,
    redirect_uri: String,
    expected_state: String,
) -> Result<sso::SsoInfo, String> {
    let provider = {
        let manager = state.sso_manager.lock().await;
        manager
            .config()
            .get_provider(&provider_name)
            .ok_or(format!("Provider {} not found", provider_name))?
            .clone()
    };

    // Generate new PKCE for the callback
    let pkce = if provider.use_pkce {
        // Note: In a real implementation, we'd need to store and retrieve the PKCE
        // from the start_sso_login call. For now we use fresh PKCE.
        None
    } else {
        None
    };

    let mut manager = state.sso_manager.lock().await;
    manager
        .wait_for_callback(&provider, &redirect_uri, &expected_state, pkce)
        .await
        .map_err(|e| e.to_string())?;

    Ok(sso::SsoInfo::from_manager(&manager))
}

/// Refresh SSO session
#[tauri::command]
async fn refresh_sso_session(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<sso::SsoInfo, String> {
    let mut manager = state.sso_manager.lock().await;
    manager.refresh_session().await.map_err(|e| e.to_string())?;
    Ok(sso::SsoInfo::from_manager(&manager))
}

/// Logout from SSO
#[tauri::command]
async fn sso_logout(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut manager = state.sso_manager.lock().await;
    manager.logout().map_err(|e| e.to_string())
}

/// Check if SSO is required for connections
#[tauri::command]
async fn is_sso_required(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, String> {
    let manager = state.sso_manager.lock().await;
    Ok(manager.config().require_sso)
}

/// Set whether SSO is required for connections
#[tauri::command]
async fn set_sso_required(
    state: tauri::State<'_, Arc<AppState>>,
    required: bool,
) -> Result<(), String> {
    let mut manager = state.sso_manager.lock().await;
    manager.set_require_sso(required).map_err(|e| e.to_string())
}

/// Set allowed email domains for SSO
#[tauri::command]
async fn set_sso_allowed_domains(
    state: tauri::State<'_, Arc<AppState>>,
    domains: Vec<String>,
) -> Result<(), String> {
    let mut manager = state.sso_manager.lock().await;
    manager.set_allowed_domains(domains).map_err(|e| e.to_string())
}

/// Get allowed email domains for SSO
#[tauri::command]
async fn get_sso_allowed_domains(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    let manager = state.sso_manager.lock().await;
    Ok(manager.config().allowed_domains.clone())
}

// ============================================================================
// License Commands
// ============================================================================

/// Get current license information
#[tauri::command]
fn get_license_info(state: tauri::State<Arc<AppState>>) -> license::LicenseInfo {
    state.license_manager.lock().license_info()
}

/// Activate a license key
#[tauri::command]
fn activate_license(
    state: tauri::State<Arc<AppState>>,
    license_key: String,
) -> Result<String, String> {
    let mut manager = state.license_manager.lock();
    match manager.activate(&license_key) {
        Ok(tier) => Ok(tier.as_str().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Deactivate current license (revert to Free)
#[tauri::command]
fn deactivate_license(state: tauri::State<Arc<AppState>>) -> Result<(), String> {
    let mut manager = state.license_manager.lock();
    manager.deactivate().map_err(|e| e.to_string())
}

/// Get current license tier
#[tauri::command]
fn get_license_tier(state: tauri::State<Arc<AppState>>) -> String {
    state.license_manager.lock().current_tier().as_str().to_string()
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    use clap::Parser;

    // Parse CLI arguments
    let cli_args = cli::Cli::parse();

    // Handle CLI commands that don't require the GUI
    if let Some(exit_code) = cli::handle_cli(&cli_args) {
        std::process::exit(exit_code);
    }

    // Handle headless listen mode
    if cli_args.listen {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let exit_code = rt.block_on(async {
            match cli::run_headless_listen(cli_args.relay.clone()).await {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            }
        });
        std::process::exit(exit_code);
    }

    // Load or create identity
    let identity = crypto::Identity::load_or_create()
        .expect("Failed to initialize identity");

    // Load or create connection config
    let connection_config = config::ConnectionConfig::load_or_create()
        .unwrap_or_default();

    // Initialize license manager with device key for encryption
    let mut license_manager = license::LicenseManager::new(identity.public_key());
    if let Err(e) = license_manager.load() {
        eprintln!("[LICENSE] Failed to load license: {}", e);
    }

    // Use relay from CLI if provided
    let relay_addresses = if let Some(ref relay) = cli_args.relay {
        relay.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        RELAY_SERVERS.iter().map(|s| s.to_string()).collect()
    };

    // Initialize SSO manager
    let sso_manager = sso::SsoManager::new()
        .expect("Failed to initialize SSO manager");

    let app_state = Arc::new(AppState {
        identity: SyncMutex::new(identity),
        host_session: AsyncMutex::new(None),
        client_sessions: AsyncMutex::new(HashMap::new()),
        active_session_id: SyncMutex::new(None),
        session_counter: AtomicU64::new(0),
        relay_addresses: SyncMutex::new(relay_addresses),
        connection_config: SyncMutex::new(connection_config),
        license_manager: SyncMutex::new(license_manager),
        clipboard_manager: clipboard::ClipboardManager::new(),
        recording_manager: recording::RecordingManager::new(),
        sso_manager: AsyncMutex::new(sso_manager),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .setup(|app| {
            // Create tray menu
            let show_item = MenuItem::with_id(app, "show", "Show SecureDesk", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            // Create tray icon
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => std::process::exit(0),
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Minimize to tray instead of closing
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_device_id,
            regenerate_device_id,
            set_relay_address,
            start_host_listener,
            connect_to_remote,
            disconnect_session,
            disconnect_all_sessions,
            set_black_screen,
            set_input_block,
            send_mouse,
            send_key,
            send_resolution,
            request_video_frame,
            respond_to_connection,
            // Multi-session commands
            list_sessions,
            set_active_session,
            get_active_session,
            get_session_count,
            // P2P commands
            get_p2p_enabled,
            set_p2p_enabled,
            get_connection_type,
            is_device_trusted,
            add_trusted_device,
            remove_trusted_device,
            get_trusted_devices,
            get_license_info,
            activate_license,
            deactivate_license,
            get_license_tier,
            get_settings,
            set_setting_bool,
            set_setting_string,
            set_setting_number,
            // Clipboard commands
            get_local_clipboard,
            set_local_clipboard,
            send_clipboard_to_remote,
            request_remote_clipboard,
            get_clipboard_sync_enabled,
            set_clipboard_sync_enabled,
            // Recording commands
            start_recording,
            stop_recording,
            is_recording,
            get_recording_status,
            list_recordings,
            delete_recording,
            open_recordings_folder,
            // SSO/OIDC commands
            get_sso_info,
            list_sso_providers,
            add_sso_provider,
            remove_sso_provider,
            start_sso_login,
            complete_sso_login,
            refresh_sso_session,
            sso_logout,
            is_sso_required,
            set_sso_required,
            set_sso_allowed_domains,
            get_sso_allowed_domains,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
