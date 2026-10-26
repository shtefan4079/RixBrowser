
#![deny(unsafe_code)]

use crate::error::{MediaError, MediaResult};
use crate::sample::DecodedAudioFrame;
use std::collections::VecDeque;

pub const DEFAULT_MAX_BUFFERED_AUDIO_SECONDS: f64 = 2.0;

#[derive(Debug)]
pub struct AudioRenderer {
    queue: VecDeque<DecodedAudioFrame>,
    current_frame_offset: usize,
    volume: f32,
    muted: bool,
    max_buffered_seconds: f64,
    total_samples_consumed: usize,
    last_pts: Option<f64>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
}

impl Default for AudioRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioRenderer {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            current_frame_offset: 0,
            volume: 1.0,
            muted: false,
            max_buffered_seconds: DEFAULT_MAX_BUFFERED_AUDIO_SECONDS,
            total_samples_consumed: 0,
            last_pts: None,
            sample_rate: None,
            channels: None,
        }
    }

    pub fn set_max_buffered_seconds(&mut self, seconds: f64) {
        self.max_buffered_seconds = seconds.max(0.1);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn buffered_duration(&self) -> f64 {
        let mut total_samples = 0usize;
        let mut rate = self.sample_rate.unwrap_or(48000) as usize;
        let mut ch = self.channels.unwrap_or(2) as usize;

        for (i, frame) in self.queue.iter().enumerate() {
            rate = frame.sample_rate as usize;
            ch = frame.channels as usize;
            let len = frame.samples_f32.len();
            if i == 0 {
                total_samples += len.saturating_sub(self.current_frame_offset);
            } else {
                total_samples += len;
            }
        }

        if rate == 0 || ch == 0 {
            0.0
        } else {
            (total_samples / ch) as f64 / rate as f64
        }
    }

    pub fn is_full(&self) -> bool {
        self.buffered_duration() >= self.max_buffered_seconds
    }

    pub fn push_frame(&mut self, frame: DecodedAudioFrame) -> MediaResult<()> {
        if self.is_full() {
            return Err(MediaError::Backpressure("Audio buffer capacity reached".into()));
        }

        if self.sample_rate.is_none() {
            self.sample_rate = Some(frame.sample_rate);
            self.channels = Some(frame.channels);
        }

        self.queue.push_back(frame);
        Ok(())
    }

    pub fn consume_samples(&mut self, max_samples: usize) -> Option<(Vec<f32>, f64)> {
        if self.queue.is_empty() {
            return None;
        }

        let mut output = Vec::with_capacity(max_samples);
        let mut start_pts = None;
        let effective_volume = if self.muted { 0.0 } else { self.volume };

        while output.len() < max_samples && !self.queue.is_empty() {
            let frame = self.queue.front().unwrap();

            if start_pts.is_none() {
                let frame_rate = frame.sample_rate.max(1) as f64;
                let frame_ch = frame.channels.max(1) as f64;
                let offset_seconds = (self.current_frame_offset as f64 / frame_ch) / frame_rate;
                let pts = frame.pts + offset_seconds;
                start_pts = Some(pts);
                self.last_pts = Some(pts);
            }

            let available = frame.samples_f32.len() - self.current_frame_offset;
            let needed = max_samples - output.len();
            let to_copy = available.min(needed);

            for i in 0..to_copy {
                let sample = frame.samples_f32[self.current_frame_offset + i] * effective_volume;
                output.push(sample);
            }

            self.current_frame_offset += to_copy;
            self.total_samples_consumed += to_copy;

            if self.current_frame_offset >= frame.samples_f32.len() {
                self.queue.pop_front();
                self.current_frame_offset = 0;
            }
        }

        start_pts.map(|pts| (output, pts))
    }

    pub fn flush(&mut self) {
        self.queue.clear();
        self.current_frame_offset = 0;
        self.last_pts = None;
    }

    pub fn last_pts(&self) -> Option<f64> {
        self.last_pts
    }

    pub fn total_samples_consumed(&self) -> usize {
        self.total_samples_consumed
    }

    pub fn sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    pub fn channels(&self) -> Option<u16> {
        self.channels
    }
}
