
#![deny(unsafe_code)]

use crate::clock::{FrameAction, MediaClock};
use crate::error::{MediaError, MediaResult};
use crate::sample::DecodedVideoFrame;
use rix_dom::VideoFrameBitmap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_QUEUED_VIDEO_FRAMES: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrameStatus {
    pub presented_new_frame: bool,
    pub next_frame_wait: Option<Duration>,
}

#[derive(Debug)]
pub struct VideoRenderer {
    queue: VecDeque<DecodedVideoFrame>,
    active_frame: Option<DecodedVideoFrame>,
    max_queued_frames: usize,
    dropped_frames_count: u64,
    presented_frames_count: u64,
}

impl Default for VideoRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoRenderer {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            active_frame: None,
            max_queued_frames: DEFAULT_MAX_QUEUED_VIDEO_FRAMES,
            dropped_frames_count: 0,
            presented_frames_count: 0,
        }
    }

    pub fn set_max_queued_frames(&mut self, max: usize) {
        self.max_queued_frames = max.max(1);
    }

    pub fn queued_frames_count(&self) -> usize {
        self.queue.len()
    }

    pub fn is_full(&self) -> bool {
        self.queue.len() >= self.max_queued_frames
    }

    pub fn dropped_frames_count(&self) -> u64 {
        self.dropped_frames_count
    }

    pub fn presented_frames_count(&self) -> u64 {
        self.presented_frames_count
    }

    pub fn buffered_duration(&self) -> f64 {
        if self.queue.is_empty() {
            0.0
        } else {
            let first = self.queue.front().map(|f| f.pts).unwrap_or(0.0);
            let last = self.queue.back().map(|f| f.pts + f.duration).unwrap_or(first);
            (last - first).max(0.0)
        }
    }

    pub fn push_frame(&mut self, frame: DecodedVideoFrame) -> MediaResult<()> {
        if self.is_full() {
            return Err(MediaError::Backpressure("Video queue capacity reached".into()));
        }

        let insert_idx = self
            .queue
            .binary_search_by(|probe| probe.pts.partial_cmp(&frame.pts).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or_else(|idx| idx);

        self.queue.insert(insert_idx, frame);
        Ok(())
    }

    pub fn update(&mut self, clock: &MediaClock, now: Instant) -> VideoFrameStatus {
        let mut presented_new_frame = false;
        let mut next_frame_wait = None;

        while let Some(front) = self.queue.front() {
            match clock.evaluate_video_frame(front.pts, now) {
                FrameAction::DropLate => {
                    self.dropped_frames_count += 1;
                    self.queue.pop_front();
                }
                FrameAction::Present => {
                    let frame = self.queue.pop_front().unwrap();
                    self.active_frame = Some(frame);
                    self.presented_frames_count += 1;
                    presented_new_frame = true;
                }
                FrameAction::Wait(wait) => {
                    next_frame_wait = Some(wait);
                    break;
                }
            }
        }

        VideoFrameStatus {
            presented_new_frame,
            next_frame_wait,
        }
    }

    pub fn active_frame(&self) -> Option<&DecodedVideoFrame> {
        self.active_frame.as_ref()
    }

    pub fn to_dom_bitmap(&self) -> Option<VideoFrameBitmap> {
        self.active_frame.as_ref().map(|frame| VideoFrameBitmap {
            width: frame.width,
            height: frame.height,
            timestamp_seconds: frame.pts,
            rgba: frame.rgba.clone(),
        })
    }

    pub fn flush(&mut self, clear_active: bool) {
        self.queue.clear();
        if clear_active {
            self.active_frame = None;
        }
    }
}
