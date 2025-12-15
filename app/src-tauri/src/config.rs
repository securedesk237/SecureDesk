//! User configuration and preferences

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Trusted device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedDevice {
    /// Device ID (9-digit)
    pub device_id: String,
    /// Friendly name (optional)
    pub name: Option<String>,
    /// When it was trusted
    pub trusted_at: u64,
    /// Last connected time
    pub last_connected: Option<u64>,
}

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    // General settings
    #[serde(default = "default_false")]
    pub start_with_windows: bool,
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    #[serde(default = "default_true")]
    pub show_notifications: bool,

    // Connection settings
    #[serde(default = "default_true")]
    pub p2p_enabled: bool,
    #[serde(default = "default_quality")]
    pub connection_quality: String,

    // Security settings
    #[serde(default = "default_true")]
    pub require_approval: bool,
    #[serde(default = "default_false")]
    pub lock_on_disconnect: bool,
    #[serde(default = "default_zero")]
    pub session_timeout: u32,

    // Privacy settings
    #[serde(default = "default_false")]
    pub hide_from_address_book: bool,
}

fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_zero() -> u32 { 0 }
fn default_quality() -> String { "auto".to_string() }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            minimize_to_tray: true,
            show_notifications: true,
            p2p_enabled: true,
            connection_quality: "auto".to_string(),
            require_approval: true,
            lock_on_disconnect: false,
            session_timeout: 0,
            hide_from_address_book: false,
        }
    }
}

/// Connection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Whether P2P is enabled (default: true)
    /// If true: Try P2P first, fallback to relay
    /// If false: Use relay only (privacy mode)
    #[serde(default = "default_true")]
    pub p2p_enabled: bool,

    /// Trusted devices that auto-accept connections
    #[serde(default)]
    pub trusted_devices: HashMap<String, TrustedDevice>,

    /// Application settings
    #[serde(default)]
    pub settings: AppSettings,

    /// Device alias (friendly name)
    #[serde(default)]
    pub alias: Option<String>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            p2p_enabled: true, // P2P enabled by default for faster connections
            trusted_devices: HashMap::new(),
            settings: AppSettings::default(),
            alias: None,
        }
    }
}

impl ConnectionConfig {
    /// Load configuration from disk or create default
    pub fn load_or_create() -> Result<Self> {
        let path = Self::config_path()?;

        if path.exists() {
            let data = fs::read_to_string(&path)?;
            let config: ConnectionConfig = serde_json::from_str(&data)?;
            Ok(config)
        } else {
            let config = Self::default();
            config.save()?;
            Ok(config)
        }
    }

    /// Save configuration to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }

    /// Get the config file path
    fn config_path() -> Result<PathBuf> {
        #[cfg(windows)]
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        #[cfg(not(windows))]
        let base = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".config"))
            .unwrap_or_else(|_| PathBuf::from("."));

        Ok(base.join("SecureDesk").join("config.json"))
    }

    /// Set P2P enabled and save
    pub fn set_p2p_enabled(&mut self, enabled: bool) -> Result<()> {
        self.p2p_enabled = enabled;
        self.save()
    }

    /// Get the device alias
    pub fn get_alias(&self) -> Option<&String> {
        self.alias.as_ref()
    }

    /// Set the device alias and save
    pub fn set_alias(&mut self, alias: &str) -> Result<()> {
        self.alias = if alias.is_empty() { None } else { Some(alias.to_string()) };
        self.save()
    }

    /// Check if a device is trusted
    pub fn is_trusted(&self, device_id: &str) -> bool {
        let clean_id = device_id.replace(' ', "");
        self.trusted_devices.contains_key(&clean_id)
    }

    /// Add a trusted device
    pub fn add_trusted_device(&mut self, device_id: &str, name: Option<String>) -> Result<()> {
        let clean_id = device_id.replace(' ', "");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.trusted_devices.insert(clean_id.clone(), TrustedDevice {
            device_id: clean_id,
            name,
            trusted_at: now,
            last_connected: Some(now),
        });
        self.save()
    }

    /// Remove a trusted device
    pub fn remove_trusted_device(&mut self, device_id: &str) -> Result<()> {
        let clean_id = device_id.replace(' ', "");
        self.trusted_devices.remove(&clean_id);
        self.save()
    }

    /// Update last connected time for a device
    pub fn update_last_connected(&mut self, device_id: &str) -> Result<()> {
        let clean_id = device_id.replace(' ', "");
        if let Some(device) = self.trusted_devices.get_mut(&clean_id) {
            device.last_connected = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
            self.save()?;
        }
        Ok(())
    }

    /// Get all trusted devices
    pub fn get_trusted_devices(&self) -> Vec<&TrustedDevice> {
        self.trusted_devices.values().collect()
    }

    /// Get all settings
    pub fn get_settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Update a setting and save
    pub fn update_setting(&mut self, key: &str, value: SettingValue) -> Result<()> {
        match key {
            "start_with_windows" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.start_with_windows = v;
                }
            }
            "minimize_to_tray" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.minimize_to_tray = v;
                }
            }
            "show_notifications" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.show_notifications = v;
                }
            }
            "p2p_enabled" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.p2p_enabled = v;
                    self.p2p_enabled = v; // Keep in sync
                }
            }
            "connection_quality" => {
                if let SettingValue::String(v) = value {
                    self.settings.connection_quality = v;
                }
            }
            "require_approval" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.require_approval = v;
                }
            }
            "lock_on_disconnect" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.lock_on_disconnect = v;
                }
            }
            "session_timeout" => {
                if let SettingValue::Number(v) = value {
                    self.settings.session_timeout = v;
                }
            }
            "hide_from_address_book" => {
                if let SettingValue::Bool(v) = value {
                    self.settings.hide_from_address_book = v;
                }
            }
            _ => {}
        }
        self.save()
    }
}

/// Setting value types
#[derive(Debug, Clone)]
pub enum SettingValue {
    Bool(bool),
    String(String),
    Number(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ConnectionConfig::default();
        assert!(config.p2p_enabled); // P2P should be enabled by default
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = ConnectionConfig { p2p_enabled: false };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: ConnectionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.p2p_enabled, loaded.p2p_enabled);
    }
}
