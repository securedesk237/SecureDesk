//! Session recording module
//! Records remote desktop sessions for later playback

use anyhow::Result;
use std::fs::{self, File};
use std::io::{BufWriter, Write, Read, BufReader};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH, Duration, Instant};
use parking_lot::Mutex;

/// Recording file format version
const RECORDING_VERSION: u8 = 1;

/// Recording file header magic bytes
const MAGIC: &[u8; 4] = b"SDRC"; // SecureDesk Recording

/// Maximum recording size (2 GB)
const MAX_RECORDING_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Recording frame types
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum FrameType {
    Video = 0x01,
    Audio = 0x02,
    Input = 0x03,
    Metadata = 0x04,
}

/// Recording metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingMetadata {
    pub version: u8,
    pub created_at: u64,
    pub remote_device_id: String,
    pub remote_device_name: String,
    pub duration_ms: u64,
    pub frame_count: u64,
    pub width: u16,
    pub height: u16,
}

/// Session recorder
pub struct SessionRecorder {
    file: Option<BufWriter<File>>,
    path: PathBuf,
    start_time: Instant,
    frame_count: u64,
    bytes_written: u64,
    metadata: RecordingMetadata,
    is_recording: bool,
}

impl SessionRecorder {
    /// Create a new session recorder
    pub fn new(remote_device_id: &str, remote_device_name: &str) -> Result<Self> {
        let recordings_dir = Self::recordings_directory()?;
        fs::create_dir_all(&recordings_dir)?;

        // Generate unique filename with timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let filename = format!("recording_{}_{}.sdrec",
            remote_device_id.replace(' ', ""),
            timestamp
        );
        let path = recordings_dir.join(&filename);

        let metadata = RecordingMetadata {
            version: RECORDING_VERSION,
            created_at: timestamp,
            remote_device_id: remote_device_id.to_string(),
            remote_device_name: remote_device_name.to_string(),
            duration_ms: 0,
            frame_count: 0,
            width: 0,
            height: 0,
        };

        Ok(Self {
            file: None,
            path,
            start_time: Instant::now(),
            frame_count: 0,
            bytes_written: 0,
            metadata,
            is_recording: false,
        })
    }

    /// Get recordings directory
    pub fn recordings_directory() -> Result<PathBuf> {
        // Use environment variables for cross-platform data directory
        #[cfg(windows)]
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
            .map_err(|_| anyhow::anyhow!("Cannot find data directory"))?;

        #[cfg(target_os = "macos")]
        let base = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
            .map_err(|_| anyhow::anyhow!("Cannot find data directory"))?;

        #[cfg(target_os = "linux")]
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
            .map_err(|_| anyhow::anyhow!("Cannot find data directory"))?;

        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        let base = std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| anyhow::anyhow!("Cannot find data directory"))?;

        Ok(base.join("SecureDesk").join("recordings"))
    }

    /// Start recording
    pub fn start(&mut self) -> Result<()> {
        if self.is_recording {
            return Ok(());
        }

        let file = File::create(&self.path)?;
        let mut writer = BufWriter::new(file);

        // Write header
        writer.write_all(MAGIC)?;
        writer.write_all(&[RECORDING_VERSION])?;

        // Reserve space for metadata (will be updated on stop)
        // Write placeholder metadata length (4 bytes) and metadata
        let metadata_json = serde_json::to_vec(&self.metadata)?;
        writer.write_all(&(metadata_json.len() as u32).to_le_bytes())?;
        writer.write_all(&metadata_json)?;

        self.bytes_written = 5 + 4 + metadata_json.len() as u64;
        self.file = Some(writer);
        self.start_time = Instant::now();
        self.is_recording = true;

        println!("[RECORDING] Started recording to {:?}", self.path);
        Ok(())
    }

    /// Stop recording and finalize file
    pub fn stop(&mut self) -> Result<PathBuf> {
        if !self.is_recording {
            anyhow::bail!("Not recording");
        }

        self.is_recording = false;

        // Update metadata with final values
        self.metadata.duration_ms = self.start_time.elapsed().as_millis() as u64;
        self.metadata.frame_count = self.frame_count;

        // Close file
        if let Some(mut writer) = self.file.take() {
            writer.flush()?;
        }

        // Re-open and update metadata at the beginning
        self.update_metadata_in_file()?;

        println!("[RECORDING] Stopped recording. Frames: {}, Duration: {}ms, Size: {} bytes",
            self.frame_count, self.metadata.duration_ms, self.bytes_written);

        Ok(self.path.clone())
    }

    /// Update metadata in the recording file
    fn update_metadata_in_file(&self) -> Result<()> {
        use std::io::{Seek, SeekFrom};

        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(&self.path)?;

        // Skip magic and version
        file.seek(SeekFrom::Start(5))?;

        // Write updated metadata
        let metadata_json = serde_json::to_vec(&self.metadata)?;
        file.write_all(&(metadata_json.len() as u32).to_le_bytes())?;
        file.write_all(&metadata_json)?;

        Ok(())
    }

    /// Write a video frame to the recording
    pub fn write_video_frame(&mut self, width: u16, height: u16, jpeg_data: &[u8]) -> Result<()> {
        if !self.is_recording {
            return Ok(());
        }

        // Check size limit
        if self.bytes_written + jpeg_data.len() as u64 > MAX_RECORDING_SIZE {
            self.stop()?;
            anyhow::bail!("Recording size limit reached");
        }

        // Update resolution in metadata
        if self.metadata.width == 0 {
            self.metadata.width = width;
            self.metadata.height = height;
        }

        let writer = self.file.as_mut().ok_or_else(|| anyhow::anyhow!("No file"))?;

        // Write frame header
        // [type (1)][timestamp_ms (8)][width (2)][height (2)][data_len (4)][data...]
        let timestamp_ms = self.start_time.elapsed().as_millis() as u64;

        writer.write_all(&[FrameType::Video as u8])?;
        writer.write_all(&timestamp_ms.to_le_bytes())?;
        writer.write_all(&width.to_le_bytes())?;
        writer.write_all(&height.to_le_bytes())?;
        writer.write_all(&(jpeg_data.len() as u32).to_le_bytes())?;
        writer.write_all(jpeg_data)?;

        self.frame_count += 1;
        self.bytes_written += 1 + 8 + 2 + 2 + 4 + jpeg_data.len() as u64;

        // Flush periodically
        if self.frame_count % 30 == 0 {
            writer.flush()?;
        }

        Ok(())
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    /// Get current recording duration
    pub fn duration(&self) -> Duration {
        if self.is_recording {
            self.start_time.elapsed()
        } else {
            Duration::from_millis(self.metadata.duration_ms)
        }
    }

    /// Get frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Get recording file path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

/// Recording info for listing
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordingInfo {
    pub path: String,
    pub filename: String,
    pub created_at: u64,
    pub duration_ms: u64,
    pub remote_device_id: String,
    pub remote_device_name: String,
    pub size_bytes: u64,
    pub frame_count: u64,
    pub resolution: String,
}

/// List all recordings
pub fn list_recordings() -> Result<Vec<RecordingInfo>> {
    let recordings_dir = SessionRecorder::recordings_directory()?;

    if !recordings_dir.exists() {
        return Ok(Vec::new());
    }

    let mut recordings = Vec::new();

    for entry in fs::read_dir(&recordings_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("sdrec") {
            continue;
        }

        if let Ok(info) = read_recording_info(&path) {
            recordings.push(info);
        }
    }

    // Sort by creation time (newest first)
    recordings.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(recordings)
}

/// Read recording info from file
fn read_recording_info(path: &PathBuf) -> Result<RecordingInfo> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    // Read and verify header
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        anyhow::bail!("Invalid recording file");
    }

    let mut version = [0u8; 1];
    reader.read_exact(&mut version)?;
    if version[0] != RECORDING_VERSION {
        anyhow::bail!("Unsupported recording version");
    }

    // Read metadata length and metadata
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let metadata_len = u32::from_le_bytes(len_buf) as usize;

    let mut metadata_buf = vec![0u8; metadata_len];
    reader.read_exact(&mut metadata_buf)?;

    let metadata: RecordingMetadata = serde_json::from_slice(&metadata_buf)?;
    let file_size = fs::metadata(path)?.len();

    Ok(RecordingInfo {
        path: path.to_string_lossy().to_string(),
        filename: path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string(),
        created_at: metadata.created_at,
        duration_ms: metadata.duration_ms,
        remote_device_id: metadata.remote_device_id,
        remote_device_name: metadata.remote_device_name,
        size_bytes: file_size,
        frame_count: metadata.frame_count,
        resolution: format!("{}x{}", metadata.width, metadata.height),
    })
}

/// Delete a recording
pub fn delete_recording(path: &str) -> Result<()> {
    let path = PathBuf::from(path);

    // Verify it's in the recordings directory
    let recordings_dir = SessionRecorder::recordings_directory()?;
    if !path.starts_with(&recordings_dir) {
        anyhow::bail!("Invalid recording path");
    }

    fs::remove_file(&path)?;
    println!("[RECORDING] Deleted recording: {:?}", path);
    Ok(())
}

/// Recording manager for use in AppState
pub struct RecordingManager {
    recorder: Mutex<Option<SessionRecorder>>,
}

impl RecordingManager {
    pub fn new() -> Self {
        Self {
            recorder: Mutex::new(None),
        }
    }

    /// Start a new recording
    pub fn start_recording(&self, remote_device_id: &str, remote_device_name: &str) -> Result<()> {
        let mut recorder_lock = self.recorder.lock();

        // Stop existing recording if any
        if let Some(ref mut existing) = *recorder_lock {
            if existing.is_recording() {
                let _ = existing.stop();
            }
        }

        let mut recorder = SessionRecorder::new(remote_device_id, remote_device_name)?;
        recorder.start()?;
        *recorder_lock = Some(recorder);
        Ok(())
    }

    /// Stop current recording
    pub fn stop_recording(&self) -> Result<PathBuf> {
        let mut recorder_lock = self.recorder.lock();

        if let Some(ref mut recorder) = *recorder_lock {
            let path = recorder.stop()?;
            *recorder_lock = None;
            Ok(path)
        } else {
            anyhow::bail!("No active recording")
        }
    }

    /// Write a video frame (called from host session)
    pub fn write_frame(&self, width: u16, height: u16, data: &[u8]) -> Result<()> {
        let mut recorder_lock = self.recorder.lock();

        if let Some(ref mut recorder) = *recorder_lock {
            recorder.write_video_frame(width, height, data)?;
        }
        Ok(())
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        self.recorder.lock().as_ref().map(|r| r.is_recording()).unwrap_or(false)
    }

    /// Get recording status
    pub fn status(&self) -> Option<RecordingStatus> {
        let recorder_lock = self.recorder.lock();
        recorder_lock.as_ref().and_then(|r| {
            if r.is_recording() {
                Some(RecordingStatus {
                    duration_ms: r.duration().as_millis() as u64,
                    frame_count: r.frame_count(),
                    path: r.path().to_string_lossy().to_string(),
                })
            } else {
                None
            }
        })
    }
}

impl Default for RecordingManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Recording status info
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordingStatus {
    pub duration_ms: u64,
    pub frame_count: u64,
    pub path: String,
}
