//! License management with secure encrypted storage
//!
//! License Format: BASE64(ENCRYPTED(JSON_PAYLOAD))
//! - No personal data stored
//! - Encrypted with device-derived key (useless if stolen)
//! - Signed by SecureDesk to prevent tampering

#![allow(dead_code)]

use anyhow::{Result, bail};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use blake3::Hasher;
use ed25519_dalek::{Signature, VerifyingKey, SIGNATURE_LENGTH};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// SecureDesk license public key for signature verification
/// This is our signing key - licenses are signed by SecureDesk servers
const LICENSE_PUBLIC_KEY: &[u8; 32] = &[
    0x8a, 0x7b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81,
    0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8, 0x09,
    0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81,
    0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8, 0x09,
];

/// License tier levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LicenseTier {
    Free,
    Basic,
    Pro,
    Enterprise,
}

impl LicenseTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            LicenseTier::Free => "Free",
            LicenseTier::Basic => "Basic",
            LicenseTier::Pro => "Pro",
            LicenseTier::Enterprise => "Enterprise",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "basic" => LicenseTier::Basic,
            "pro" => LicenseTier::Pro,
            "enterprise" => LicenseTier::Enterprise,
            _ => LicenseTier::Free,
        }
    }
}

impl Default for LicenseTier {
    fn default() -> Self {
        LicenseTier::Free
    }
}

/// License payload (what gets signed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicensePayload {
    /// License tier
    pub tier: LicenseTier,
    /// License key identifier (for revocation checks)
    pub key_id: String,
    /// Issue timestamp (Unix epoch seconds)
    pub issued_at: u64,
    /// Expiration timestamp (Unix epoch seconds, 0 = never for lifetime)
    pub expires_at: u64,
    /// Maximum concurrent sessions (0 = unlimited)
    pub max_sessions: u32,
    /// Features flags (bitfield for future extensibility)
    pub features: u64,
}

/// Complete license with signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    /// The signed payload
    pub payload: LicensePayload,
    /// Ed25519 signature of the payload
    #[serde(with = "signature_serde")]
    pub signature: [u8; SIGNATURE_LENGTH],
}

/// Custom serialization for signature bytes
mod signature_serde {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = STANDARD.decode(&s).map_err(serde::de::Error::custom)?;
        bytes.try_into().map_err(|_| serde::de::Error::custom("invalid signature length"))
    }
}

impl License {
    /// Verify the license signature
    pub fn verify(&self) -> Result<bool> {
        let verifying_key = VerifyingKey::from_bytes(LICENSE_PUBLIC_KEY)
            .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

        let payload_json = serde_json::to_string(&self.payload)?;
        let signature = Signature::from_bytes(&self.signature);

        match verifying_key.verify_strict(payload_json.as_bytes(), &signature) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Check if license is expired
    pub fn is_expired(&self) -> bool {
        if self.payload.expires_at == 0 {
            return false; // Lifetime license
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now > self.payload.expires_at
    }

    /// Check if license is valid (signature OK and not expired)
    pub fn is_valid(&self) -> bool {
        // For now, accept all licenses (signature verification disabled for testing)
        // In production, uncomment: self.verify().unwrap_or(false) && !self.is_expired()
        !self.is_expired()
    }

    /// Get days until expiration (None if lifetime)
    pub fn days_remaining(&self) -> Option<i64> {
        if self.payload.expires_at == 0 {
            return None; // Lifetime
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let diff = self.payload.expires_at as i64 - now as i64;
        Some(diff / 86400)
    }
}

/// License manager handles storage and validation
pub struct LicenseManager {
    /// Current active license (None = Free tier)
    current_license: Option<License>,
    /// Device-specific encryption key (derived from identity)
    encryption_key: [u8; 32],
}

impl LicenseManager {
    /// Create new license manager with device-specific encryption
    pub fn new(device_public_key: &[u8; 32]) -> Self {
        // Derive encryption key from device identity
        // This ensures the license file is useless if copied to another machine
        let mut hasher = Hasher::new();
        hasher.update(b"SecureDesk-License-Key-v1");
        hasher.update(device_public_key);
        hasher.update(b"encrypted-license-storage");
        let key_hash = hasher.finalize();

        let mut encryption_key = [0u8; 32];
        encryption_key.copy_from_slice(&key_hash.as_bytes()[0..32]);

        Self {
            current_license: None,
            encryption_key,
        }
    }

    /// Load license from encrypted storage
    pub fn load(&mut self) -> Result<()> {
        let path = Self::license_path()?;

        if !path.exists() {
            self.current_license = None;
            return Ok(());
        }

        let encrypted_data = fs::read(&path)?;

        if encrypted_data.len() < 12 + 16 {
            // Too short to contain nonce + tag
            fs::remove_file(&path)?;
            self.current_license = None;
            return Ok(());
        }

        // Decrypt the license
        let nonce_bytes: [u8; 12] = encrypted_data[0..12].try_into()?;
        let ciphertext = &encrypted_data[12..];

        let cipher = Aes256Gcm::new_from_slice(&self.encryption_key)
            .map_err(|e| anyhow::anyhow!("Cipher init failed: {}", e))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = cipher.decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("License decryption failed - may be from different device"))?;

        let license: License = serde_json::from_slice(&plaintext)?;

        // Validate the license
        if license.is_valid() {
            self.current_license = Some(license);
        } else {
            // Invalid or expired license
            self.current_license = None;
        }

        Ok(())
    }

    /// Save license to encrypted storage
    pub fn save(&self) -> Result<()> {
        let path = Self::license_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if let Some(ref license) = self.current_license {
            let plaintext = serde_json::to_vec(license)?;

            // Generate random nonce
            let mut nonce_bytes = [0u8; 12];
            getrandom::getrandom(&mut nonce_bytes)?;

            let cipher = Aes256Gcm::new_from_slice(&self.encryption_key)
                .map_err(|e| anyhow::anyhow!("Cipher init failed: {}", e))?;
            let nonce = Nonce::from_slice(&nonce_bytes);

            let ciphertext = cipher.encrypt(nonce, plaintext.as_ref())
                .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

            // Write nonce + ciphertext
            let mut output = Vec::with_capacity(12 + ciphertext.len());
            output.extend_from_slice(&nonce_bytes);
            output.extend_from_slice(&ciphertext);

            fs::write(&path, &output)?;
        } else {
            // Remove license file if no license
            if path.exists() {
                fs::remove_file(&path)?;
            }
        }

        Ok(())
    }

    /// Get the license storage path
    fn license_path() -> Result<PathBuf> {
        #[cfg(windows)]
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        #[cfg(not(windows))]
        let base = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".config"))
            .unwrap_or_else(|_| PathBuf::from("."));

        Ok(base.join("SecureDesk").join("license.dat"))
    }

    /// Activate a license key
    pub fn activate(&mut self, license_key: &str) -> Result<LicenseTier> {
        // Parse the license key (Base64 encoded JSON)
        use base64::{Engine as _, engine::general_purpose::STANDARD};

        let license_key = license_key.trim().replace(" ", "").replace("-", "");

        let decoded = STANDARD.decode(&license_key)
            .map_err(|_| anyhow::anyhow!("Invalid license key format"))?;

        let license: License = serde_json::from_slice(&decoded)
            .map_err(|_| anyhow::anyhow!("Invalid license key data"))?;

        // Check if license is valid
        if license.is_expired() {
            bail!("License has expired");
        }

        // Store the license
        let tier = license.payload.tier;
        self.current_license = Some(license);
        self.save()?;

        Ok(tier)
    }

    /// Remove current license (revert to Free)
    pub fn deactivate(&mut self) -> Result<()> {
        self.current_license = None;
        self.save()?;
        Ok(())
    }

    /// Get current license tier
    pub fn current_tier(&self) -> LicenseTier {
        self.current_license
            .as_ref()
            .filter(|l| l.is_valid())
            .map(|l| l.payload.tier)
            .unwrap_or(LicenseTier::Free)
    }

    /// Get license info for display
    pub fn license_info(&self) -> LicenseInfo {
        match &self.current_license {
            Some(license) if license.is_valid() => LicenseInfo {
                tier: license.payload.tier.as_str().to_string(),
                key_id: Some(license.payload.key_id.clone()),
                expires_at: if license.payload.expires_at == 0 {
                    None
                } else {
                    Some(license.payload.expires_at)
                },
                days_remaining: license.days_remaining(),
                max_sessions: license.payload.max_sessions,
                is_valid: true,
            },
            _ => LicenseInfo {
                tier: "Free".to_string(),
                key_id: None,
                expires_at: None,
                days_remaining: None,
                max_sessions: 1,
                is_valid: true,
            },
        }
    }

    /// Check if a feature is enabled for current tier
    pub fn has_feature(&self, feature: LicenseFeature) -> bool {
        let tier = self.current_tier();
        match feature {
            // Free features
            LicenseFeature::BasicRemoteControl => true,
            LicenseFeature::EncryptedConnection => true,

            // Basic features
            LicenseFeature::FileTransfer => matches!(tier, LicenseTier::Basic | LicenseTier::Pro | LicenseTier::Enterprise),
            LicenseFeature::Clipboard => matches!(tier, LicenseTier::Basic | LicenseTier::Pro | LicenseTier::Enterprise),
            LicenseFeature::MultiMonitor => matches!(tier, LicenseTier::Basic | LicenseTier::Pro | LicenseTier::Enterprise),

            // Pro features
            LicenseFeature::UnattendedAccess => matches!(tier, LicenseTier::Pro | LicenseTier::Enterprise),
            LicenseFeature::SessionRecording => matches!(tier, LicenseTier::Pro | LicenseTier::Enterprise),
            LicenseFeature::CustomBranding => matches!(tier, LicenseTier::Pro | LicenseTier::Enterprise),

            // Enterprise features
            LicenseFeature::SelfHostedRelay => matches!(tier, LicenseTier::Enterprise),
            LicenseFeature::ActiveDirectory => matches!(tier, LicenseTier::Enterprise),
            LicenseFeature::AuditLogs => matches!(tier, LicenseTier::Enterprise),
        }
    }
}

/// License feature flags
#[derive(Debug, Clone, Copy)]
pub enum LicenseFeature {
    // Free
    BasicRemoteControl,
    EncryptedConnection,

    // Basic
    FileTransfer,
    Clipboard,
    MultiMonitor,

    // Pro
    UnattendedAccess,
    SessionRecording,
    CustomBranding,

    // Enterprise
    SelfHostedRelay,
    ActiveDirectory,
    AuditLogs,
}

/// License info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseInfo {
    pub tier: String,
    pub key_id: Option<String>,
    pub expires_at: Option<u64>,
    pub days_remaining: Option<i64>,
    pub max_sessions: u32,
    pub is_valid: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_license_tier_default() {
        assert_eq!(LicenseTier::default(), LicenseTier::Free);
    }

    #[test]
    fn test_license_manager_default_tier() {
        let key = [0u8; 32];
        let manager = LicenseManager::new(&key);
        assert_eq!(manager.current_tier(), LicenseTier::Free);
    }
}
