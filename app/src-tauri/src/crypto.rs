//! Cryptographic identity and secure sessions

#![allow(dead_code)]

use anyhow::Result;
use blake3::Hasher;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use snow::{Builder, HandshakeState, TransportState};
use std::fs;
use std::path::PathBuf;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};

const NOISE_PATTERN: &str = "Noise_XK_25519_ChaChaPoly_BLAKE2s";

/// Device identity - stored locally, never sent to servers
#[derive(Clone)]
pub struct Identity {
    x25519_secret: X25519Secret,
    x25519_public: X25519Public,
    ed25519_key: SigningKey,
}

impl Identity {
    /// Generate new random identity
    pub fn generate() -> Self {
        let x25519_secret = X25519Secret::random_from_rng(OsRng);
        let x25519_public = X25519Public::from(&x25519_secret);
        let ed25519_key = SigningKey::generate(&mut OsRng);

        Self {
            x25519_secret,
            x25519_public,
            ed25519_key,
        }
    }

    /// Load from disk or create new
    pub fn load_or_create() -> Result<Self> {
        let path = Self::identity_path()?;

        if path.exists() {
            Self::load(&path)
        } else {
            let identity = Self::generate();
            identity.save(&path)?;
            Ok(identity)
        }
    }

    fn load(path: &PathBuf) -> Result<Self> {
        let data = fs::read(path)?;
        if data.len() != 64 {
            anyhow::bail!("Invalid identity file");
        }

        let x25519_bytes: [u8; 32] = data[0..32].try_into()?;
        let x25519_secret = X25519Secret::from(x25519_bytes);
        let x25519_public = X25519Public::from(&x25519_secret);

        let ed25519_bytes: [u8; 32] = data[32..64].try_into()?;
        let ed25519_key = SigningKey::from_bytes(&ed25519_bytes);

        Ok(Self {
            x25519_secret,
            x25519_public,
            ed25519_key,
        })
    }

    fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(self.x25519_secret.as_bytes());
        data.extend_from_slice(self.ed25519_key.as_bytes());
        fs::write(path, &data)?;
        Ok(())
    }

    fn identity_path() -> Result<PathBuf> {
        #[cfg(windows)]
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        #[cfg(not(windows))]
        let base = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".config"))
            .unwrap_or_else(|_| PathBuf::from("."));

        Ok(base.join("SecureDesk").join("identity.key"))
    }

    /// Get device ID (shown to user for sharing)
    /// Format: XXX XXX XXX (9 digits)
    pub fn device_id(&self) -> String {
        let mut hasher = Hasher::new();
        hasher.update(self.x25519_public.as_bytes());
        hasher.update(self.ed25519_key.verifying_key().as_bytes());
        let hash = hasher.finalize();

        // Convert first bytes to digits
        let bytes = hash.as_bytes();
        let num = u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
        ]) % 1_000_000_000;

        let id = format!("{:09}", num);
        format!("{} {} {}", &id[0..3], &id[3..6], &id[6..9])
    }

    /// Get raw device ID (no spaces)
    pub fn device_id_raw(&self) -> String {
        self.device_id().replace(' ', "")
    }

    /// Regenerate identity (new keys, new device ID)
    pub fn regenerate() -> Result<Self> {
        let path = Self::identity_path()?;

        // Delete existing identity file if it exists
        if path.exists() {
            fs::remove_file(&path)?;
        }

        // Generate and save new identity
        let identity = Self::generate();
        identity.save(&path)?;
        Ok(identity)
    }

    /// Get X25519 public key bytes
    pub fn public_key(&self) -> &[u8; 32] {
        self.x25519_public.as_bytes()
    }

    /// Create Noise initiator (client connecting to host)
    pub fn create_initiator(&self, remote_public: &[u8]) -> Result<HandshakeState> {
        let builder = Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(self.x25519_secret.as_bytes())
            .remote_public_key(remote_public)
            .build_initiator()?;
        Ok(builder)
    }

    /// Create Noise responder (host accepting connection)
    pub fn create_responder(&self) -> Result<HandshakeState> {
        let builder = Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(self.x25519_secret.as_bytes())
            .build_responder()?;
        Ok(builder)
    }
}

/// Secure transport after Noise handshake completes
pub struct SecureChannel {
    transport: TransportState,
}

impl SecureChannel {
    pub fn from_handshake(handshake: HandshakeState) -> Result<Self> {
        let transport = handshake.into_transport_mode()?;
        Ok(Self { transport })
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut ciphertext = vec![0u8; plaintext.len() + 16];
        let len = self.transport.write_message(plaintext, &mut ciphertext)?;
        ciphertext.truncate(len);
        Ok(ciphertext)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut plaintext = vec![0u8; ciphertext.len()];
        let len = self.transport.read_message(ciphertext, &mut plaintext)?;
        plaintext.truncate(len);
        Ok(plaintext)
    }
}
