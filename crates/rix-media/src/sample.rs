
#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackType {
    Audio,
    Video,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u16,
    pub codec: String,
    pub extra_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoConfig {
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub extra_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaTrack {
    pub id: u32,
    pub track_type: TrackType,
    pub timescale: u64,
    pub duration: Option<f64>,
    pub audio_config: Option<AudioConfig>,
    pub video_config: Option<VideoConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaSample {
    pub track_id: u32,
    pub pts: f64,
    pub dts: f64,
    pub duration: f64,
    pub is_sync_point: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudioFrame {
    pub pts: f64,
    pub channels: u16,
    pub sample_rate: u32,
    pub samples_f32: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedVideoFrame {
    pub pts: f64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub rgba: Vec<u8>,
}
