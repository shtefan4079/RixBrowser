
#![deny(unsafe_code)]

use crate::audio_sink::AudioRenderer;
use crate::clock::MediaClock;
use crate::engine::session::MediaSession;
use crate::error::{MediaError, MediaResult};
use crate::video_sink::VideoRenderer;
use rix_dom::VideoFrameBitmap;
use rix_ipc::media::{MediaSessionId, MediaWorkerCommand, MediaWorkerEvent, SourceBufferId};
use std::time::Instant;

pub const MAX_SOURCE_BUFFER_BYTES: usize = 32 * 1024 * 1024;

pub const MAX_SESSION_BYTES: usize = 64 * 1024 * 1024;

pub const EVICTION_SAFETY_WINDOW_SECONDS: f64 = 30.0;

#[derive(Debug, Clone)]
pub struct EngineTickOutput {
    pub current_position: f64,
    pub active_video_frame: Option<VideoFrameBitmap>,
    pub audio_samples: Option<(Vec<f32>, f64)>,
    pub is_ended: bool,
    pub events: Vec<MediaWorkerEvent>,
}

pub struct PlaybackEngine {
    session: MediaSession,
    clock: MediaClock,
    audio_renderer: AudioRenderer,
    video_renderer: VideoRenderer,
    end_of_stream: bool,
    has_dispatched_ended: bool,
}

impl PlaybackEngine {
    pub fn new(session_id: MediaSessionId, generation_id: u64) -> Self {
        Self {
            session: MediaSession::new(session_id, generation_id),
            clock: MediaClock::new(),
            audio_renderer: AudioRenderer::new(),
            video_renderer: VideoRenderer::new(),
            end_of_stream: false,
            has_dispatched_ended: false,
        }
    }

    pub fn session_id(&self) -> MediaSessionId {
        self.session.session_id()
    }

    pub fn clock(&self) -> &MediaClock {
        &self.clock
    }

    pub fn clock_mut(&mut self) -> &mut MediaClock {
        &mut self.clock
    }

    pub fn audio_renderer(&self) -> &AudioRenderer {
        &self.audio_renderer
    }

    pub fn audio_renderer_mut(&mut self) -> &mut AudioRenderer {
        &mut self.audio_renderer
    }

    pub fn video_renderer(&self) -> &VideoRenderer {
        &self.video_renderer
    }

    pub fn video_renderer_mut(&mut self) -> &mut VideoRenderer {
        &mut self.video_renderer
    }

    pub fn add_source_buffer(&mut self, buffer_id: SourceBufferId, mime_type: String) -> MediaResult<()> {
        self.session.add_source_buffer(buffer_id, mime_type)
    }

    pub fn remove_source_buffer(&mut self, buffer_id: SourceBufferId) {
        self.session.remove_source_buffer(buffer_id);
    }

    pub fn append_segment(
        &mut self,
        buffer_id: SourceBufferId,
        append_id: u64,
        data: &[u8],
        now: Instant,
    ) -> MediaResult<Vec<MediaWorkerEvent>> {
        let chunk_size = data.len();

        let buffer_usage = self.session.buffer_bytes(buffer_id);
        let session_usage = self.session.total_session_bytes();

        if buffer_usage + chunk_size > MAX_SOURCE_BUFFER_BYTES || session_usage + chunk_size > MAX_SESSION_BYTES {
            let current_pos = self.clock.current_position(now);
            let eviction_cutoff = (current_pos - EVICTION_SAFETY_WINDOW_SECONDS).max(0.0);
            self.session.evict_coded_frames(eviction_cutoff);

            let new_buffer_usage = self.session.buffer_bytes(buffer_id);
            let new_session_usage = self.session.total_session_bytes();
            if new_buffer_usage + chunk_size > MAX_SOURCE_BUFFER_BYTES
                || new_session_usage + chunk_size > MAX_SESSION_BYTES
            {
                return Err(MediaError::QuotaExceeded(format!(
                    "Buffer exceeded memory quota: buffer_bytes={}, session_bytes={}, max_buffer={}, max_session={}",
                    new_buffer_usage + chunk_size,
                    new_session_usage + chunk_size,
                    MAX_SOURCE_BUFFER_BYTES,
                    MAX_SESSION_BYTES
                )));
            }
        }

        let events = self.session.append_segment(buffer_id, append_id, data)?;

        if let Some(dur) = self.session.duration() {
            self.clock.set_duration(Some(dur));
        }

        self.drain_decoded_frames();

        Ok(events)
    }

    pub fn push_sample(
        &mut self,
        sample: crate::sample::MediaSample,
        codec: &str,
        extra_data: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> MediaResult<Vec<MediaWorkerEvent>> {
        let events = self.session.push_sample(sample, codec, extra_data, sample_rate, channels)?;
        self.drain_decoded_frames();
        if let Some(dur) = self.session.duration() {
            self.clock.set_duration(Some(dur));
        }
        Ok(events)
    }

    fn drain_decoded_frames(&mut self) {
        while !self.audio_renderer.is_full() {
            if let Some(frame) = self.session.pop_audio_frame() {
                if let Err(error) = self.audio_renderer.push_frame(frame) {
                    tracing::warn!(%error, "media: audio renderer rejected a decoded frame");
                    break;
                }
            } else {
                break;
            }
        }

        while !self.video_renderer.is_full() {
            if let Some(frame) = self.session.pop_video_frame() {
                if let Err(error) = self.video_renderer.push_frame(frame) {
                    tracing::warn!(%error, "media: video renderer rejected a decoded frame");
                    break;
                }
            } else {
                break;
            }
        }
    }

    pub fn play(&mut self, now: Instant) {
        self.clock.play(now);
        self.has_dispatched_ended = false;
    }

    pub fn pause(&mut self, now: Instant) {
        self.clock.pause(now);
    }

    pub fn set_playback_rate(&mut self, rate: f64, now: Instant) {
        self.clock.set_playback_rate(rate, now);
    }

    pub fn set_volume(&mut self, volume: f32, muted: bool) {
        self.audio_renderer.set_volume(volume);
        self.audio_renderer.set_muted(muted);
    }

    pub fn seek(&mut self, target_time: f64, now: Instant) -> MediaResult<Vec<MediaWorkerEvent>> {
        self.audio_renderer.flush();
        self.video_renderer.flush(false);

        self.session.seek(target_time)?;

        self.clock.seek(target_time, now);
        self.end_of_stream = false;
        self.has_dispatched_ended = false;

        self.drain_decoded_frames();

        self.video_renderer.update(&self.clock, now);

        Ok(vec![MediaWorkerEvent::SeekCompleted {
            session_id: self.session_id(),
            current_time: self.clock.current_position(now),
        }])
    }

    pub fn set_end_of_stream(&mut self, is_ended: bool) {
        self.end_of_stream = is_ended;
    }

    pub fn tick(&mut self, now: Instant) -> EngineTickOutput {
        self.drain_decoded_frames();

        let audio_samples = if !self.clock.is_paused() {
            if let Some((samples, pts)) = self.audio_renderer.consume_samples(1024) {
                if let Some(rate) = self.audio_renderer.sample_rate() {
                    let channels = self.audio_renderer.channels().unwrap_or(2).max(1) as usize;
                    let frames_count = samples.len() / channels;
                    self.clock.sync_audio(pts, frames_count, rate, now);
                }
                Some((samples, pts))
            } else {
                None
            }
        } else {
            None
        };

        self.video_renderer.update(&self.clock, now);
        let active_video_frame = self.video_renderer.to_dom_bitmap();

        let current_position = self.clock.current_position(now);
        let mut is_ended = false;
        let mut events = Vec::new();

        if self.end_of_stream {
            if let Some(dur) = self.clock.duration() {
                if current_position >= dur && !self.has_dispatched_ended {
                    is_ended = true;
                    self.has_dispatched_ended = true;
                    self.clock.pause(now);
                    events.push(MediaWorkerEvent::PlaybackEnded {
                        session_id: self.session_id(),
                    });
                }
            }
        }

        EngineTickOutput {
            current_position,
            active_video_frame,
            audio_samples,
            is_ended,
            events,
        }
    }

    pub fn handle_command(&mut self, cmd: MediaWorkerCommand, now: Instant) -> MediaResult<Vec<MediaWorkerEvent>> {
        match cmd {
            MediaWorkerCommand::OpenMediaSession { .. } => {
                Ok(vec![MediaWorkerEvent::SessionOpened {
                    session_id: self.session_id(),
                }])
            }
            MediaWorkerCommand::CloseMediaSession { .. } => {
                self.shutdown();
                Ok(Vec::new())
            }
            MediaWorkerCommand::AddSourceBuffer { buffer_id, mime_type, .. } => {
                self.add_source_buffer(buffer_id, mime_type)?;
                Ok(vec![MediaWorkerEvent::SourceBufferAdded {
                    session_id: self.session_id(),
                    buffer_id,
                }])
            }
            MediaWorkerCommand::RemoveSourceBuffer { buffer_id, .. } => {
                self.remove_source_buffer(buffer_id);
                Ok(Vec::new())
            }
            MediaWorkerCommand::AppendSegment { buffer_id, append_id, data, .. } => {
                self.append_segment(buffer_id, append_id, &data, now)
            }
            MediaWorkerCommand::AbortAppend { buffer_id, .. } => {
                Ok(vec![MediaWorkerEvent::AppendAborted {
                    session_id: self.session_id(),
                    buffer_id,
                }])
            }
            MediaWorkerCommand::EndOfStream { .. } => {
                self.set_end_of_stream(true);
                Ok(Vec::new())
            }
            MediaWorkerCommand::Seek { target_time, .. } => {
                self.seek(target_time, now)
            }
            MediaWorkerCommand::ResetGeneration { generation_id, .. } => {
                self.session.reset_generation(generation_id);
                self.audio_renderer.flush();
                self.video_renderer.flush(true);
                self.clock.seek(0.0, now);
                Ok(Vec::new())
            }
            MediaWorkerCommand::Play { playback_rate, .. } => {
                self.set_playback_rate(playback_rate, now);
                self.play(now);
                Ok(Vec::new())
            }
            MediaWorkerCommand::Pause { .. } => {
                self.pause(now);
                Ok(Vec::new())
            }
            MediaWorkerCommand::SetPlaybackRate { playback_rate, .. } => {
                self.set_playback_rate(playback_rate, now);
                Ok(Vec::new())
            }
            MediaWorkerCommand::SetVolume { volume, muted, .. } => {
                self.set_volume(volume, muted);
                Ok(Vec::new())
            }
        }
    }

    pub fn shutdown(&mut self) {
        self.clock = MediaClock::new();
        self.audio_renderer.flush();
        self.video_renderer.flush(true);
        self.session.reset_generation(0);
        self.end_of_stream = false;
        self.has_dispatched_ended = false;
    }
}
