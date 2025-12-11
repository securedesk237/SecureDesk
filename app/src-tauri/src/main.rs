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

use parking_lot::Mutex as SyncMutex;
use std::sync::Arc;
use tauri::{
    CustomMenuItem, Manager, SystemTray, SystemTrayEvent,
    SystemTrayMenu, SystemTrayMenuItem, WindowEvent,
};
use tokio::sync::Mutex as AsyncMutex;

/// Relay server addresses (multiple for failover and load balancing)
const RELAY_SERVERS: &[&str] = &[
    "relay.securedesk.one:8443",
    "relay2.securedesk.one:8443",
];

/// Global application state
struct AppState {
    identity: SyncMutex<crypto::Identity>,
    host_session: AsyncMutex<Option<host::HostSession>>,
    client_session: AsyncMutex<Option<client::ClientSession>>,
    relay_addresses: SyncMutex<Vec<String>>,
    connection_config: SyncMutex<config::ConnectionConfig>,
    license_manager: SyncMutex<license::LicenseManager>,
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

/// Connect to a remote device (client mode)
/// Tries each relay server until one works
#[tauri::command]
async fn connect_to_remote(
    state: tauri::State<'_, Arc<AppState>>,
    remote_id: String,
) -> Result<(), String> {
    let relays = state.relay_addresses.lock().clone();
    let identity = state.identity.lock().clone();

    let mut last_error = String::from("No relay servers configured");

    for relay in relays {
        match client::ClientSession::connect(relay.clone(), remote_id.clone(), identity.clone()).await {
            Ok(session) => {
                *state.client_session.lock().await = Some(session);
                return Ok(());
            }
            Err(e) => {
                last_error = format!("Relay {} failed: {}", relay, e);
                continue;
            }
        }
    }

    Err(last_error)
}

/// Disconnect current session
#[tauri::command]
async fn disconnect_session(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    // Take session out of lock first, then await
    let session = state.client_session.lock().await.take();
    if let Some(session) = session {
        session.disconnect().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Toggle black screen on remote (when in client mode)
#[tauri::command]
async fn set_black_screen(
    state: tauri::State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    // Take session out temporarily, operate on it, put it back
    let mut session_opt = state.client_session.lock().await.take();
    if let Some(ref mut session) = session_opt {
        let result = session.set_black_screen(enabled).await;
        *state.client_session.lock().await = session_opt;
        result.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Toggle input blocking on remote (when in client mode)
#[tauri::command]
async fn set_input_block(
    state: tauri::State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    let mut session_opt = state.client_session.lock().await.take();
    if let Some(ref mut session) = session_opt {
        let result = session.set_input_block(enabled).await;
        *state.client_session.lock().await = session_opt;
        result.map_err(|e| e.to_string())?;
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
) -> Result<(), String> {
    let mut session_opt = state.client_session.lock().await.take();
    if let Some(ref mut session) = session_opt {
        let result = session.send_mouse(x, y, &event_type, button).await;
        *state.client_session.lock().await = session_opt;
        result.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Send key event to remote
#[tauri::command]
async fn send_key(
    state: tauri::State<'_, Arc<AppState>>,
    key_code: u16,
    pressed: bool,
) -> Result<(), String> {
    let mut session_opt = state.client_session.lock().await.take();
    if let Some(ref mut session) = session_opt {
        let result = session.send_key(key_code, pressed).await;
        *state.client_session.lock().await = session_opt;
        result.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Send client viewport resolution to host for adaptive scaling
#[tauri::command]
async fn send_resolution(
    state: tauri::State<'_, Arc<AppState>>,
    width: u16,
    height: u16,
) -> Result<(), String> {
    let mut session_opt = state.client_session.lock().await.take();
    if let Some(ref mut session) = session_opt {
        let result = session.send_resolution(width, height).await;
        *state.client_session.lock().await = session_opt;
        result.map_err(|e| e.to_string())?;
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
) -> Result<Option<VideoFrame>, String> {
    let mut session_opt = state.client_session.lock().await.take();
    if let Some(ref mut session) = session_opt {
        let result = session.request_and_receive_frame().await;
        *state.client_session.lock().await = session_opt;
        match result {
            Ok(Some((width, height, data))) => {
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

/// Get current connection type (for active sessions)
#[tauri::command]
async fn get_connection_type(state: tauri::State<'_, Arc<AppState>>) -> Result<String, String> {
    // Check client session first
    let client_opt = state.client_session.lock().await;
    if let Some(ref session) = *client_opt {
        return Ok(session.connection_type().to_string());
    }
    drop(client_opt);

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

    let app_state = Arc::new(AppState {
        identity: SyncMutex::new(identity),
        host_session: AsyncMutex::new(None),
        client_session: AsyncMutex::new(None),
        relay_addresses: SyncMutex::new(
            RELAY_SERVERS.iter().map(|s| s.to_string()).collect()
        ),
        connection_config: SyncMutex::new(connection_config),
        license_manager: SyncMutex::new(license_manager),
    });

    // System tray
    let show = CustomMenuItem::new("show", "Show SecureDesk");
    let quit = CustomMenuItem::new("quit", "Quit");
    let tray_menu = SystemTrayMenu::new()
        .add_item(show)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit);

    tauri::Builder::default()
        .manage(app_state)
        .system_tray(SystemTray::new().with_menu(tray_menu))
        .on_system_tray_event(|app, event| {
            match event {
                SystemTrayEvent::LeftClick { .. } => {
                    if let Some(window) = app.get_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                SystemTrayEvent::MenuItemClick { id, .. } => {
                    match id.as_str() {
                        "show" => {
                            if let Some(window) = app.get_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => std::process::exit(0),
                        _ => {}
                    }
                }
                _ => {}
            }
        })
        .on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event.event() {
                // Minimize to tray instead of closing
                let _ = event.window().hide();
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
            set_black_screen,
            set_input_block,
            send_mouse,
            send_key,
            send_resolution,
            request_video_frame,
            respond_to_connection,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
