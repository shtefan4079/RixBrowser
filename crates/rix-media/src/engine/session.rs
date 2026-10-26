
#![deny(unsafe_code)]

use crate::decoder::{AudioDecoder, CodecRegistry, VideoDecoder};
use crate::demuxer::{create_demuxer_for_mime, MediaDemuxer};
use crate::error::{MediaError, MediaResult};
use crate::sample::{DecodedAudioFrame, DecodedVideoFrame, MediaSample, TrackType};
use rix_ipc::media::{MediaSessionId, MediaWorkerEvent, SourceBufferId};
use std::collections::HashMap;

struct StreamPipeline {
    mime_type: String,
    demuxer: Box<dyn MediaDemuxer>,
    audio_decoders: HashMap<u32, Box<dyn AudioDecoder>>,
    video_decoders: HashMap<u32, Box<dyn VideoDecoder>>,
    total_appended_bytes: usize,
}

impl StreamPipeline {
    fn new(mime_type: String) -> MediaResult<Self> {
        let demuxer = create_demuxer_for_mime(&mime_type).ok_or_else(|| {
            MediaError::UnsupportedContainer(format!("No demuxer found for MIME: {}", mime_type))
        })?;

        Ok(Self {
            mime_type: mime_type,
            demuxer,
            audio_decoders: HashMap::new(),
            video_decoders: HashMap::new(),
            total_appended_bytes: 0,
        })
    }

    fn update_decoders(&mut self) {
        if !self.demuxer.is_init_complete() {
            return;
        }

        for track in self.demuxer.tracks() {
            match track.track_type {
                TrackType::Audio => {
                    if !self.audio_decoders.contains_key(&track.id) {
                        if let Some(ref cfg) = track.audio_config {
                            if let Some(decoder) = CodecRegistry::create_audio_decoder(
                                &cfg.codec,
                                cfg.sample_rate,
                                cfg.channels,
                                &cfg.extra_data,
                            ) {
                                self.audio_decoders.insert(track.id, decoder);
                            }
                        }
                    }
                }
                TrackType::Video => {
                    if !self.video_decoders.contains_key(&track.id) {
                        if let Some(ref cfg) = track.video_config {
                            if let Some(decoder) = CodecRegistry::create_video_decoder(&cfg.codec, &cfg.extra_data) {
                                self.video_decoders.insert(track.id, decoder);
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct MediaSession {
    session_id: MediaSessionId,
    generation_id: u64,
    pipelines: HashMap<SourceBufferId, StreamPipeline>,
    decoded_audio_queue: Vec<DecodedAudioFrame>,
    decoded_video_queue: Vec<DecodedVideoFrame>,
}

impl MediaSession {
    pub fn new(session_id: MediaSessionId, generation_id: u64) -> Self {
        Self {
            session_id,
            generation_id,
            pipelines: HashMap::new(),
            decoded_audio_queue: Vec::new(),
            decoded_video_queue: Vec::new(),
        }
    }

    pub fn session_id(&self) -> MediaSessionId {
        self.session_id
    }

    pub fn generation_id(&self) -> u64 {
        self.generation_id
    }

    pub fn total_session_bytes(&self) -> usize {
        self.pipelines.values().map(|p| p.total_appended_bytes).sum()
    }

    pub fn buffer_bytes(&self, buffer_id: SourceBufferId) -> usize {
        self.pipelines.get(&buffer_id).map(|p| p.total_appended_bytes).unwrap_or(0)
    }

    pub fn duration(&self) -> Option<f64> {
        let mut max_dur: Option<f64> = None;
        for pipeline in self.pipelines.values() {
            if let Some(d) = pipeline.demuxer.duration() {
                max_dur = Some(max_dur.map_or(d, |curr| curr.max(d)));
            }
        }
        max_dur
    }

    pub fn evict_coded_frames(&mut self, before_pts: f64) -> usize {
        let mut evicted = 0;
        let before_audio_len = self.decoded_audio_queue.len();
        self.decoded_audio_queue.retain(|f| f.pts >= before_pts);
        evicted += before_audio_len - self.decoded_audio_queue.len();

        let before_video_len = self.decoded_video_queue.len();
        self.decoded_video_queue.retain(|f| f.pts >= before_pts);
        evicted += before_video_len - self.decoded_video_queue.len();

        for pipeline in self.pipelines.values_mut() {
            let before_count = pipeline.demuxer.buffered_ranges().len();
            pipeline.demuxer.remove_range(0.0, before_pts);
            let after_count = pipeline.demuxer.buffered_ranges().len();
            evicted += before_count.saturating_sub(after_count);
        }
        evicted
    }

    pub fn add_source_buffer(
        &mut self,
        buffer_id: SourceBufferId,
        mime_type: String,
    ) -> MediaResult<()> {
        let pipeline = StreamPipeline::new(mime_type)?;
        self.pipelines.insert(buffer_id, pipeline);
        Ok(())
    }

    pub fn remove_source_buffer(&mut self, buffer_id: SourceBufferId) {
        self.pipelines.remove(&buffer_id);
    }

    pub fn append_segment(
        &mut self,
        buffer_id: SourceBufferId,
        append_id: u64,
        data: &[u8],
    ) -> MediaResult<Vec<MediaWorkerEvent>> {
        let pipeline = self
            .pipelines
            .get_mut(&buffer_id)
            .ok_or_else(|| MediaError::InvalidState(format!("Buffer {:?} not found", buffer_id)))?;

        pipeline.total_appended_bytes += data.len();
        pipeline.demuxer.append_data(data)?;
        pipeline.update_decoders();

        let mut events = Vec::new();

        while let Some(sample) = pipeline.demuxer.poll_sample() {
            let track_id = sample.track_id;

            if let Some(audio_dec) = pipeline.audio_decoders.get_mut(&track_id) {
                if let Ok(frames) = audio_dec.decode_packet(&sample) {
                    for frame in frames {
                        events.push(MediaWorkerEvent::AudioSamplesReady {
                            session_id: self.session_id,
                            pts: frame.pts,
                            sample_rate: frame.sample_rate,
                            channels: frame.channels,
                            samples_count: frame.samples_f32.len(),
                        });
                        self.decoded_audio_queue.push(frame);
                    }
                }
            } else if let Some(video_dec) = pipeline.video_decoders.get_mut(&track_id) {
                if let Ok(Some(frame)) = video_dec.decode_packet(&sample) {
                    events.push(MediaWorkerEvent::VideoFrameReady {
                        session_id: self.session_id,
                        pts: frame.pts,
                        duration: frame.duration,
                        width: frame.width,
                        height: frame.height,
                        stride: frame.stride,
                        buffer_index: None,
                    });
                    self.decoded_video_queue.push(frame);
                }
            }
        }

        let buffered_ranges = pipeline.demuxer.buffered_ranges();
        let duration = pipeline.demuxer.duration();

        events.push(MediaWorkerEvent::AppendCompleted {
            session_id: self.session_id,
            buffer_id,
            append_id,
            buffered_ranges,
            duration,
        });

        Ok(events)
    }

    pub fn push_sample(
        &mut self,
        sample: MediaSample,
        codec: &str,
        extra_data: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> MediaResult<Vec<MediaWorkerEvent>> {
        let track_id = sample.track_id;

        if self.pipelines.is_empty() {
            let _ = self.add_source_buffer(SourceBufferId(1), "video/mp4".to_string());
        }
        let pipeline_keys: Vec<SourceBufferId> = self.pipelines.keys().copied().collect();
        let mut chosen: Option<SourceBufferId> = None;
        for key in &pipeline_keys {
            if let Some(p) = self.pipelines.get(key) {
                if p.audio_decoders.contains_key(&track_id)
                    || p.video_decoders.contains_key(&track_id)
                    || p.demuxer.tracks().iter().any(|t| t.id == track_id)
                {
                    chosen = Some(*key);
                    break;
                }
            }
        }
        if chosen.is_none() {
            chosen = pipeline_keys.first().copied();
        }
        let buffer_id = chosen.ok_or_else(|| {
            MediaError::InvalidState("No source buffer available for pushed sample".into())
        })?;

        let pipeline = self
            .pipelines
            .get_mut(&buffer_id)
            .ok_or_else(|| MediaError::InvalidState(format!("Buffer {:?} not found", buffer_id)))?;

        if !pipeline.audio_decoders.contains_key(&track_id)
            && !pipeline.video_decoders.contains_key(&track_id)
        {
            let is_video = codec == "avc1" || codec == "h264" || codec == "avc"
                || codec == "vp8" || codec == "vp9" || codec == "vp09";
            let is_audio = codec == "mp4a" || codec == "aac" || codec == "opus"
                || codec == "vorbis" || codec.contains("pcm");
            if is_video {
                if let Some(dec) = CodecRegistry::create_video_decoder(codec, extra_data) {
                    pipeline.video_decoders.insert(track_id, dec);
                }
            }
            if is_audio {
                let sr = if sample_rate > 0 { sample_rate } else if pipeline.mime_type.contains("webm") { 48000 } else { 44100 };
                let ch = if channels > 0 { channels } else { 2 };
                if let Some(dec) = CodecRegistry::create_audio_decoder(codec, sr, ch, extra_data) {
                    pipeline.audio_decoders.insert(track_id, dec);
                }
            }
        }

        let mut events = Vec::new();
        let mut handled = false;

        if let Some(dec) = pipeline.video_decoders.get_mut(&track_id) {
            match dec.decode_packet(&sample) {
                Ok(Some(frame)) => {
                    events.push(MediaWorkerEvent::VideoFrameReady {
                        session_id: self.session_id,
                        pts: frame.pts,
                        duration: frame.duration,
                        width: frame.width,
                        height: frame.height,
                        stride: frame.stride,
                        buffer_index: None,
                    });
                    self.decoded_video_queue.push(frame);
                    handled = true;
                }
                Ok(None) => {
                    handled = true;
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        track = track_id,
                        codec = %codec,
                        "media: video decoder failed on a sample"
                    );
                    return Err(MediaError::DecodeError(
                        "push_sample",
                        format!("{codec} decoder failed: {error}"),
                    ));
                }
            }
        } else if let Some(dec) = pipeline.audio_decoders.get_mut(&track_id) {
            match dec.decode_packet(&sample) {
                Ok(frames) => {
                    handled = true;
                    for frame in frames {
                        events.push(MediaWorkerEvent::AudioSamplesReady {
                            session_id: self.session_id,
                            pts: frame.pts,
                            sample_rate: frame.sample_rate,
                            channels: frame.channels,
                            samples_count: frame.samples_f32.len(),
                        });
                        self.decoded_audio_queue.push(frame);
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        track = track_id,
                        codec = %codec,
                        "media: audio decoder failed on a sample"
                    );
                    return Err(MediaError::DecodeError(
                        "push_sample",
                        format!("{codec} decoder failed: {error}"),
                    ));
                }
            }
        }

        if !handled {
            return Err(MediaError::DecodeError(
                "push_sample",
                format!("no decoder is registered for track {track_id} ({codec})"),
            ));
        }

        Ok(events)
    }

    pub fn seek(&mut self, target_time: f64) -> MediaResult<()> {
        for pipeline in self.pipelines.values_mut() {
            pipeline.demuxer.seek(target_time)?;
            for dec in pipeline.audio_decoders.values_mut() {
                dec.reset();
            }
            for dec in pipeline.video_decoders.values_mut() {
                dec.reset();
            }
        }
        self.decoded_audio_queue.retain(|f| f.pts >= target_time);
        self.decoded_video_queue.retain(|f| f.pts >= target_time);
        Ok(())
    }

    pub fn reset_generation(&mut self, generation_id: u64) {
        self.generation_id = generation_id;
        for pipeline in self.pipelines.values_mut() {
            pipeline.demuxer.reset();
            for dec in pipeline.audio_decoders.values_mut() {
                dec.reset();
            }
            for dec in pipeline.video_decoders.values_mut() {
                dec.reset();
            }
        }
        self.decoded_audio_queue.clear();
        self.decoded_video_queue.clear();
    }

    pub fn decoded_audio_count(&self) -> usize {
        self.decoded_audio_queue.len()
    }

    pub fn decoded_video_count(&self) -> usize {
        self.decoded_video_queue.len()
    }

    pub fn pop_audio_frame(&mut self) -> Option<DecodedAudioFrame> {
        if !self.decoded_audio_queue.is_empty() {
            Some(self.decoded_audio_queue.remove(0))
        } else {
            None
        }
    }

    pub fn pop_video_frame(&mut self) -> Option<DecodedVideoFrame> {
        if !self.decoded_video_queue.is_empty() {
            Some(self.decoded_video_queue.remove(0))
        } else {
            None
        }
    }
}
