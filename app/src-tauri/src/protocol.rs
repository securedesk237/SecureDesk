//! Wire protocol definitions

#![allow(dead_code)]

use anyhow::Result;

/// Maximum frame size (16 MB)
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Protocol channels
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Channel {
    Control = 0x00,
    Video = 0x01,
    Input = 0x02,
    Clipboard = 0x03,
    File = 0x04,
    Privacy = 0x05,
}

impl TryFrom<u8> for Channel {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0x00 => Ok(Self::Control),
            0x01 => Ok(Self::Video),
            0x02 => Ok(Self::Input),
            0x03 => Ok(Self::Clipboard),
            0x04 => Ok(Self::File),
            0x05 => Ok(Self::Privacy),
            _ => anyhow::bail!("Invalid channel: {}", value),
        }
    }
}

/// Protocol frame
#[derive(Debug, Clone)]
pub struct Frame {
    pub channel: Channel,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(channel: Channel, payload: Vec<u8>) -> Self {
        Self { channel, payload }
    }

    pub fn control(msg_type: u8, data: &[u8]) -> Self {
        let mut payload = vec![msg_type];
        payload.extend_from_slice(data);
        Self::new(Channel::Control, payload)
    }

    pub fn video(data: Vec<u8>) -> Self {
        Self::new(Channel::Video, data)
    }

    pub fn input(data: Vec<u8>) -> Self {
        Self::new(Channel::Input, data)
    }

    pub fn privacy(cmd: u8) -> Self {
        Self::new(Channel::Privacy, vec![cmd])
    }

    pub fn clipboard(msg_type: u8, data: &[u8]) -> Self {
        let mut payload = vec![msg_type];
        payload.extend_from_slice(data);
        Self::new(Channel::Clipboard, payload)
    }

    pub fn file(msg_type: u8, data: &[u8]) -> Self {
        let mut payload = vec![msg_type];
        payload.extend_from_slice(data);
        Self::new(Channel::File, payload)
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let len = self.payload.len();
        let mut bytes = Vec::with_capacity(4 + len);
        bytes.push(self.channel as u8);
        bytes.push((len >> 16) as u8);
        bytes.push((len >> 8) as u8);
        bytes.push(len as u8);
        bytes.extend(&self.payload);
        bytes
    }

    /// Parse from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            anyhow::bail!("Frame too short");
        }

        let channel = Channel::try_from(data[0])?;
        let len = ((data[1] as usize) << 16)
            | ((data[2] as usize) << 8)
            | (data[3] as usize);

        if len > MAX_FRAME_SIZE {
            anyhow::bail!("Frame too large");
        }

        if data.len() < 4 + len {
            anyhow::bail!("Frame incomplete");
        }

        Ok(Self {
            channel,
            payload: data[4..4 + len].to_vec(),
        })
    }
}

/// Control message types
pub mod control {
    pub const HANDSHAKE: u8 = 0x01;
    pub const SESSION_REQUEST: u8 = 0x02;
    pub const SESSION_ACCEPT: u8 = 0x03;
    pub const SESSION_END: u8 = 0x04;
    pub const KEEPALIVE: u8 = 0x05;
    pub const RESOLUTION: u8 = 0x06;    // Client sends viewport resolution

    // P2P negotiation messages
    pub const P2P_OFFER: u8 = 0x10;     // Client offers P2P with public addr
    pub const P2P_ANSWER: u8 = 0x11;    // Host responds with public addr
    pub const P2P_READY: u8 = 0x12;     // Both ready, attempt P2P
    pub const P2P_FAILED: u8 = 0x13;    // P2P failed, use relay

    pub const ERROR: u8 = 0xFF;
}

/// Input message types
pub mod input {
    pub const MOUSE_MOVE: u8 = 0x01;
    pub const MOUSE_BUTTON: u8 = 0x02;
    pub const MOUSE_SCROLL: u8 = 0x03;
    pub const KEY_DOWN: u8 = 0x04;
    pub const KEY_UP: u8 = 0x05;
}

/// Privacy message types
pub mod privacy {
    pub const BLACK_SCREEN_ON: u8 = 0x01;
    pub const BLACK_SCREEN_OFF: u8 = 0x02;
    pub const INPUT_BLOCK_ON: u8 = 0x03;
    pub const INPUT_BLOCK_OFF: u8 = 0x04;
    pub const STATUS_ACK: u8 = 0x05;
}

/// Clipboard message types
pub mod clipboard {
    /// Request remote clipboard content
    pub const CLIPBOARD_REQUEST: u8 = 0x01;
    /// Provide clipboard content (response to request or push)
    pub const CLIPBOARD_DATA: u8 = 0x02;
    /// Notify that clipboard has changed
    pub const CLIPBOARD_CHANGED: u8 = 0x03;
    /// Clipboard sync enabled/disabled notification
    pub const CLIPBOARD_SYNC_STATUS: u8 = 0x04;

    /// Clipboard data types
    pub const DATA_TYPE_TEXT: u8 = 0x01;
    pub const DATA_TYPE_IMAGE: u8 = 0x02;
    pub const DATA_TYPE_FILES: u8 = 0x03;
}

/// File transfer message types
pub mod file {
    /// Request to start file transfer
    pub const FILE_OFFER: u8 = 0x01;
    /// Accept file transfer
    pub const FILE_ACCEPT: u8 = 0x02;
    /// Reject file transfer
    pub const FILE_REJECT: u8 = 0x03;
    /// File data chunk
    pub const FILE_CHUNK: u8 = 0x04;
    /// File transfer complete
    pub const FILE_COMPLETE: u8 = 0x05;
    /// Cancel file transfer
    pub const FILE_CANCEL: u8 = 0x06;
    /// File transfer progress
    pub const FILE_PROGRESS: u8 = 0x07;
}
