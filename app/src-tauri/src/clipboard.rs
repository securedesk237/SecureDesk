//! Clipboard synchronization module
//! Handles reading/writing clipboard content across platforms

use anyhow::Result;
use parking_lot::Mutex;

/// Maximum clipboard data size (10 MB)
pub const MAX_CLIPBOARD_SIZE: usize = 10 * 1024 * 1024;

/// Clipboard data types
#[derive(Debug, Clone, PartialEq)]
pub enum ClipboardData {
    Text(String),
    Image { width: u32, height: u32, data: Vec<u8> }, // PNG data
    Files(Vec<String>), // File paths
}

impl ClipboardData {
    /// Serialize clipboard data for transmission
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ClipboardData::Text(text) => {
                let text_bytes = text.as_bytes();
                let mut data = Vec::with_capacity(5 + text_bytes.len());
                data.push(crate::protocol::clipboard::DATA_TYPE_TEXT);
                data.extend(&(text_bytes.len() as u32).to_le_bytes());
                data.extend(text_bytes);
                data
            }
            ClipboardData::Image { width, height, data: img_data } => {
                let mut data = Vec::with_capacity(13 + img_data.len());
                data.push(crate::protocol::clipboard::DATA_TYPE_IMAGE);
                data.extend(&width.to_le_bytes());
                data.extend(&height.to_le_bytes());
                data.extend(&(img_data.len() as u32).to_le_bytes());
                data.extend(img_data);
                data
            }
            ClipboardData::Files(paths) => {
                let joined = paths.join("\n");
                let path_bytes = joined.as_bytes();
                let mut data = Vec::with_capacity(5 + path_bytes.len());
                data.push(crate::protocol::clipboard::DATA_TYPE_FILES);
                data.extend(&(path_bytes.len() as u32).to_le_bytes());
                data.extend(path_bytes);
                data
            }
        }
    }

    /// Deserialize clipboard data from transmission
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            anyhow::bail!("Empty clipboard data");
        }

        let data_type = data[0];
        let payload = &data[1..];

        match data_type {
            crate::protocol::clipboard::DATA_TYPE_TEXT => {
                if payload.len() < 4 {
                    anyhow::bail!("Invalid text clipboard data");
                }
                let len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                if payload.len() < 4 + len {
                    anyhow::bail!("Incomplete text data");
                }
                let text = String::from_utf8_lossy(&payload[4..4+len]).to_string();
                Ok(ClipboardData::Text(text))
            }
            crate::protocol::clipboard::DATA_TYPE_IMAGE => {
                if payload.len() < 12 {
                    anyhow::bail!("Invalid image clipboard data");
                }
                let width = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let height = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let len = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
                if payload.len() < 12 + len {
                    anyhow::bail!("Incomplete image data");
                }
                Ok(ClipboardData::Image {
                    width,
                    height,
                    data: payload[12..12+len].to_vec(),
                })
            }
            crate::protocol::clipboard::DATA_TYPE_FILES => {
                if payload.len() < 4 {
                    anyhow::bail!("Invalid files clipboard data");
                }
                let len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                if payload.len() < 4 + len {
                    anyhow::bail!("Incomplete files data");
                }
                let paths_str = String::from_utf8_lossy(&payload[4..4+len]).to_string();
                let paths: Vec<String> = paths_str.lines().map(|s| s.to_string()).collect();
                Ok(ClipboardData::Files(paths))
            }
            _ => anyhow::bail!("Unknown clipboard data type: {}", data_type),
        }
    }

    /// Get type name for display
    pub fn type_name(&self) -> &'static str {
        match self {
            ClipboardData::Text(_) => "text",
            ClipboardData::Image { .. } => "image",
            ClipboardData::Files(_) => "files",
        }
    }
}

/// Clipboard manager for cross-platform operations
pub struct ClipboardManager {
    last_content: Mutex<Option<ClipboardData>>,
    sync_enabled: Mutex<bool>,
    last_hash: Mutex<Option<u64>>,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self {
            last_content: Mutex::new(None),
            sync_enabled: Mutex::new(true),
            last_hash: Mutex::new(None),
        }
    }

    /// Simple hash function for clipboard content
    fn compute_hash(data: &ClipboardData) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        match data {
            ClipboardData::Text(text) => {
                0u8.hash(&mut hasher);
                text.hash(&mut hasher);
            }
            ClipboardData::Image { width, height, data } => {
                1u8.hash(&mut hasher);
                width.hash(&mut hasher);
                height.hash(&mut hasher);
                data.hash(&mut hasher);
            }
            ClipboardData::Files(files) => {
                2u8.hash(&mut hasher);
                files.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Update the stored hash for duplicate detection
    pub fn update_hash(&self, data: &ClipboardData) {
        let hash = Self::compute_hash(data);
        *self.last_hash.lock() = Some(hash);
    }

    /// Check if content matches the last hash
    pub fn matches_hash(&self, data: &ClipboardData) -> bool {
        let hash = Self::compute_hash(data);
        self.last_hash.lock().map(|h| h == hash).unwrap_or(false)
    }

    /// Get current clipboard content
    pub fn get_clipboard(&self) -> Result<Option<ClipboardData>> {
        // Stub implementation - returns last content
        Ok(self.last_content.lock().clone())
    }

    /// Set clipboard content
    pub fn set_clipboard(&self, data: &ClipboardData) -> Result<()> {
        *self.last_content.lock() = Some(data.clone());
        println!("[CLIPBOARD] Set clipboard: {:?}", data.type_name());
        Ok(())
    }

    /// Check if clipboard content has changed
    pub fn has_changed(&self) -> bool {
        // Stub - always return false for now
        false
    }

    /// Get sync enabled state
    pub fn is_sync_enabled(&self) -> bool {
        *self.sync_enabled.lock()
    }

    /// Set sync enabled state
    pub fn set_sync_enabled(&self, enabled: bool) {
        *self.sync_enabled.lock() = enabled;
    }
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}
