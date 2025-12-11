//! Quality of Service (QoS) management

#![allow(dead_code)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Initial FPS when connection starts (conservative)
pub const INIT_FPS: u32 = 15;

/// Minimum FPS to maintain usability
pub const MIN_FPS: u32 = 5;

/// Maximum FPS when network is good
pub const MAX_FPS: u32 = 60;

/// RTT sample window size
const RTT_WINDOW_SIZE: usize = 60;

/// Quality levels
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QualityLevel {
    Low,      // 50% quality, higher FPS
    Balanced, // 75% quality, balanced
    Best,     // 100% quality, may reduce FPS
}

impl QualityLevel {
    pub fn jpeg_quality(&self) -> u8 {
        match self {
            QualityLevel::Low => 50,
            QualityLevel::Balanced => 75,
            QualityLevel::Best => 95,
        }
    }

    pub fn min_fps(&self) -> u32 {
        match self {
            QualityLevel::Low => 12,
            QualityLevel::Balanced => 10,
            QualityLevel::Best => 8,
        }
    }
}

/// RTT (Round-Trip Time) tracker using smoothed RTT estimation
pub struct RttTracker {
    samples: VecDeque<u32>, // RTT samples in ms
    smoothed_rtt: u32,
    min_rtt: u32,
    last_update: Instant,
}

impl RttTracker {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(RTT_WINDOW_SIZE),
            smoothed_rtt: 50, // Default 50ms
            min_rtt: u32::MAX,
            last_update: Instant::now(),
        }
    }

    /// Add a new RTT sample
    pub fn add_sample(&mut self, rtt_ms: u32) {
        // Add to window
        if self.samples.len() >= RTT_WINDOW_SIZE {
            self.samples.pop_front();
        }
        self.samples.push_back(rtt_ms);

        // Update minimum
        if rtt_ms < self.min_rtt {
            self.min_rtt = rtt_ms;
        }

        // Calculate smoothed RTT (weighted average)
        let window_min = self.samples.iter().min().copied().unwrap_or(rtt_ms);
        self.smoothed_rtt = (self.min_rtt + window_min) / 2;

        self.last_update = Instant::now();
    }

    /// Get the smoothed RTT value
    pub fn get_rtt(&self) -> u32 {
        self.smoothed_rtt
    }

    /// Check if data is stale (no updates for a while)
    pub fn is_stale(&self) -> bool {
        self.last_update.elapsed() > Duration::from_secs(5)
    }
}

/// QoS Manager for adaptive streaming
/// Manages FPS and quality based on network conditions
pub struct QosManager {
    rtt_tracker: RttTracker,
    current_fps: u32,
    target_quality: QualityLevel,
    quality_ratio: f32, // 0.0 - 1.0, multiplier for quality
    frame_times: VecDeque<Instant>,
    last_adjustment: Instant,
}

impl QosManager {
    pub fn new() -> Self {
        Self {
            rtt_tracker: RttTracker::new(),
            current_fps: INIT_FPS,
            target_quality: QualityLevel::Balanced,
            quality_ratio: 1.0,
            frame_times: VecDeque::with_capacity(60),
            last_adjustment: Instant::now(),
        }
    }

    /// Set target quality level
    pub fn set_quality(&mut self, quality: QualityLevel) {
        self.target_quality = quality;
    }

    /// Record a new RTT measurement
    pub fn record_rtt(&mut self, rtt_ms: u32) {
        self.rtt_tracker.add_sample(rtt_ms);

        // Adjust every 500ms to avoid oscillation
        if self.last_adjustment.elapsed() > Duration::from_millis(500) {
            self.adjust_parameters();
            self.last_adjustment = Instant::now();
        }
    }

    /// Record that a frame was sent (for FPS calculation)
    pub fn record_frame(&mut self) {
        let now = Instant::now();

        // Remove old samples (older than 1 second)
        while let Some(front) = self.frame_times.front() {
            if now.duration_since(*front) > Duration::from_secs(1) {
                self.frame_times.pop_front();
            } else {
                break;
            }
        }

        self.frame_times.push_back(now);
    }

    /// Get actual FPS based on recent frames
    pub fn get_actual_fps(&self) -> u32 {
        self.frame_times.len() as u32
    }

    /// Adjust FPS and quality based on network conditions
    fn adjust_parameters(&mut self) {
        let rtt = self.rtt_tracker.get_rtt();
        let min_fps = self.target_quality.min_fps();

        // FPS adjustment based on RTT
        if rtt < 50 {
            // Excellent network - increase FPS aggressively
            self.current_fps = (self.current_fps + 5).min(MAX_FPS);
            self.quality_ratio = (self.quality_ratio + 0.15).min(1.0);
        } else if rtt < 100 {
            // Good network - moderate FPS increase
            self.current_fps = (self.current_fps + 2).min(MAX_FPS);
            self.quality_ratio = (self.quality_ratio + 0.10).min(1.0);
        } else if rtt < 150 {
            // Acceptable network - maintain minimum FPS
            self.current_fps = self.current_fps.max(min_fps);
            self.quality_ratio = (self.quality_ratio + 0.05).min(1.0);
        } else if rtt < 200 {
            // Poor network - reduce FPS slightly
            self.current_fps = (self.current_fps.saturating_sub(2)).max(min_fps);
            self.quality_ratio = (self.quality_ratio * 0.95).max(0.5);
        } else if rtt < 300 {
            // Bad network - reduce more
            self.current_fps = (self.current_fps.saturating_sub(5)).max(MIN_FPS);
            self.quality_ratio = (self.quality_ratio * 0.90).max(0.4);
        } else {
            // Very bad network - minimum settings
            self.current_fps = MIN_FPS;
            self.quality_ratio = (self.quality_ratio * 0.85).max(0.3);
        }
    }

    /// Get the target FPS
    pub fn get_target_fps(&self) -> u32 {
        self.current_fps
    }

    /// Get the frame interval in milliseconds
    pub fn get_frame_interval_ms(&self) -> u64 {
        1000 / self.current_fps as u64
    }

    /// Get the effective JPEG quality (base quality * ratio)
    pub fn get_jpeg_quality(&self) -> u8 {
        let base = self.target_quality.jpeg_quality() as f32;
        (base * self.quality_ratio) as u8
    }

    /// Get current network quality description
    pub fn get_network_quality(&self) -> &'static str {
        let rtt = self.rtt_tracker.get_rtt();
        if rtt < 50 {
            "Excellent"
        } else if rtt < 100 {
            "Good"
        } else if rtt < 150 {
            "Fair"
        } else if rtt < 300 {
            "Poor"
        } else {
            "Bad"
        }
    }

    /// Get debug stats
    pub fn get_stats(&self) -> QosStats {
        QosStats {
            rtt_ms: self.rtt_tracker.get_rtt(),
            target_fps: self.current_fps,
            actual_fps: self.get_actual_fps(),
            quality_ratio: self.quality_ratio,
            jpeg_quality: self.get_jpeg_quality(),
            network_quality: self.get_network_quality(),
        }
    }
}

impl Default for QosManager {
    fn default() -> Self {
        Self::new()
    }
}

/// QoS statistics for debugging/display
#[derive(Debug, Clone)]
pub struct QosStats {
    pub rtt_ms: u32,
    pub target_fps: u32,
    pub actual_fps: u32,
    pub quality_ratio: f32,
    pub jpeg_quality: u8,
    pub network_quality: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtt_tracker() {
        let mut tracker = RttTracker::new();

        // Add some samples
        for rtt in [50, 55, 48, 52, 51] {
            tracker.add_sample(rtt);
        }

        // Should have a reasonable smoothed value
        let rtt = tracker.get_rtt();
        assert!(rtt >= 48 && rtt <= 55);
    }

    #[test]
    fn test_qos_adjustment() {
        let mut qos = QosManager::new();

        // Simulate excellent network
        for _ in 0..10 {
            qos.record_rtt(30);
        }

        // FPS should increase
        assert!(qos.get_target_fps() > INIT_FPS);

        // Simulate poor network
        for _ in 0..10 {
            qos.record_rtt(300);
        }

        // FPS should decrease
        assert!(qos.get_target_fps() < MAX_FPS);
    }
}
