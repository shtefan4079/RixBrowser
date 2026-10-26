
#![deny(unsafe_code)]

use std::time::{Duration, Instant};

pub const LATE_FRAME_DROP_THRESHOLD_SECONDS: f64 = 0.150;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameAction {
    Present,
    Wait(Duration),
    DropLate,
}

#[derive(Debug, Clone, Copy)]
struct AudioAnchor {
    pts: f64,
    samples_consumed: u64,
    sample_rate: u32,
    anchor_instant: Instant,
}

#[derive(Debug, Clone)]
pub struct MediaClock {
    paused: bool,
    playback_rate: f64,
    position_seconds: f64,
    wall_anchor: Option<Instant>,
    audio_anchor: Option<AudioAnchor>,
    duration: Option<f64>,
}

impl Default for MediaClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaClock {
    pub fn new() -> Self {
        Self {
            paused: true,
            playback_rate: 1.0,
            position_seconds: 0.0,
            wall_anchor: None,
            audio_anchor: None,
            duration: None,
        }
    }

    pub fn set_duration(&mut self, duration: Option<f64>) {
        self.duration = duration.filter(|d| *d >= 0.0);
    }

    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn playback_rate(&self) -> f64 {
        self.playback_rate
    }

    pub fn set_playback_rate(&mut self, rate: f64, now: Instant) {
        if self.playback_rate == rate || !rate.is_finite() || rate <= 0.0 {
            return;
        }
        self.position_seconds = self.current_position(now);
        self.playback_rate = rate.clamp(0.0625, 16.0);
        if !self.paused {
            self.wall_anchor = Some(now);
            if let Some(ref mut audio) = self.audio_anchor {
                audio.anchor_instant = now;
            }
        }
    }

    pub fn play(&mut self, now: Instant) {
        if !self.paused {
            return;
        }
        self.paused = false;
        self.wall_anchor = Some(now);
        if let Some(ref mut audio) = self.audio_anchor {
            audio.anchor_instant = now;
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if self.paused {
            return;
        }
        self.position_seconds = self.current_position(now);
        self.paused = true;
        self.wall_anchor = None;
        if let Some(ref mut audio) = self.audio_anchor {
            audio.anchor_instant = now;
        }
    }

    pub fn seek(&mut self, target_seconds: f64, now: Instant) {
        let mut target = target_seconds.max(0.0);
        if let Some(dur) = self.duration {
            target = target.min(dur);
        }
        self.position_seconds = target;
        self.audio_anchor = None;
        if !self.paused {
            self.wall_anchor = Some(now);
        } else {
            self.wall_anchor = None;
        }
    }

    pub fn sync_audio(&mut self, pts: f64, samples_consumed: usize, sample_rate: u32, now: Instant) {
        if sample_rate == 0 {
            return;
        }
        let audio_pos = pts + (samples_consumed as f64 / sample_rate as f64);
        let current = self.current_position(now);
        let drift = (audio_pos - current).abs();

        if drift > 0.040 {
            self.position_seconds = audio_pos;
            if !self.paused {
                self.wall_anchor = Some(now);
            }
        }

        self.audio_anchor = Some(AudioAnchor {
            pts,
            samples_consumed: samples_consumed as u64,
            sample_rate,
            anchor_instant: now,
        });
    }

    pub fn current_position(&self, now: Instant) -> f64 {
        if self.paused {
            return self.position_seconds;
        }

        let pos = if let Some(ref audio) = self.audio_anchor {
            let elapsed = now.saturating_duration_since(audio.anchor_instant).as_secs_f64();
            let base_audio = audio.pts + (audio.samples_consumed as f64 / audio.sample_rate as f64);
            base_audio + (elapsed * self.playback_rate)
        } else if let Some(anchor) = self.wall_anchor {
            let elapsed = now.saturating_duration_since(anchor).as_secs_f64();
            self.position_seconds + (elapsed * self.playback_rate)
        } else {
            self.position_seconds
        };

        if let Some(dur) = self.duration {
            pos.clamp(0.0, dur)
        } else {
            pos.max(0.0)
        }
    }

    pub fn evaluate_video_frame(&self, frame_pts: f64, now: Instant) -> FrameAction {
        let current = self.current_position(now);

        if frame_pts < current - LATE_FRAME_DROP_THRESHOLD_SECONDS {
            FrameAction::DropLate
        } else if frame_pts <= current {
            FrameAction::Present
        } else {
            let delta = (frame_pts - current) / self.playback_rate;
            FrameAction::Wait(Duration::from_secs_f64(delta.max(0.0)))
        }
    }
}
