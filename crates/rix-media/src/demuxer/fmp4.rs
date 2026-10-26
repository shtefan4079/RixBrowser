
#![deny(unsafe_code)]

use super::traits::{merge_time_ranges, MediaDemuxer};
use crate::error::{MediaError, MediaResult};
use crate::sample::{AudioConfig, MediaSample, MediaTrack, TrackType, VideoConfig};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Default)]
struct TrackDefaults {
    _default_sample_description_index: u32,
    default_sample_duration: u32,
    default_sample_size: u32,
    default_sample_flags: u32,
}

#[derive(Debug, Clone)]
struct ParsedRunSample {
    duration: u32,
    size: u32,
    flags: u32,
    cto: i32,
}

#[derive(Debug, Clone)]
struct PendingFragmentRun {
    track_id: u32,
    base_decode_time: u64,
    samples: Vec<ParsedRunSample>,
}

pub struct Fmp4Demuxer {
    buffer: Vec<u8>,
    tracks: Vec<MediaTrack>,
    track_defaults: HashMap<u32, TrackDefaults>,
    duration_sec: Option<f64>,
    movie_timescale: u32,
    is_init_done: bool,
    pending_runs: Vec<PendingFragmentRun>,
    samples_queue: VecDeque<MediaSample>,
    buffered: Vec<(f64, f64)>,
}

impl Fmp4Demuxer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(64 * 1024),
            tracks: Vec::new(),
            track_defaults: HashMap::new(),
            duration_sec: None,
            movie_timescale: 1000,
            is_init_done: false,
            pending_runs: Vec::new(),
            samples_queue: VecDeque::new(),
            buffered: Vec::new(),
        }
    }

    fn parse_boxes(&mut self) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= self.buffer.len() {
            let box_size_raw = u32::from_be_bytes([
                self.buffer[cursor],
                self.buffer[cursor + 1],
                self.buffer[cursor + 2],
                self.buffer[cursor + 3],
            ]) as usize;

            let box_type = [
                self.buffer[cursor + 4],
                self.buffer[cursor + 5],
                self.buffer[cursor + 6],
                self.buffer[cursor + 7],
            ];

            let (header_len, box_len) = if box_size_raw == 1 {
                if cursor + 16 > self.buffer.len() {
                    break;
                }
                let mut size_bytes = [0u8; 8];
                size_bytes.copy_from_slice(&self.buffer[cursor + 8..cursor + 16]);
                let large_size = u64::from_be_bytes(size_bytes) as usize;
                (16, large_size)
            } else if box_size_raw == 0 {
                break;
            } else {
                (8, box_size_raw)
            };

            if box_len < header_len {
                return Err(MediaError::InvalidBox(format!(
                    "Box length {} is smaller than header {}",
                    box_len, header_len
                )));
            }

            if cursor + box_len > self.buffer.len() {
                break;
            }

            let payload_start = cursor + header_len;
            let payload_end = cursor + box_len;
            let payload = self.buffer[payload_start..payload_end].to_vec();

            match &box_type {
                b"moov" => {
                    self.parse_moov(&payload)?;
                    self.is_init_done = true;
                }
                b"moof" => {
                    self.parse_moof(&payload)?;
                }
                b"mdat" => {
                    self.parse_mdat(&payload)?;
                }
                _ => {
                }
            }

            cursor += box_len;
        }

        if cursor > 0 {
            self.buffer.drain(..cursor);
        }

        Ok(())
    }

    fn parse_moov(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            match box_type {
                b"mvhd" => self.parse_mvhd(child)?,
                b"trak" => self.parse_trak(child)?,
                b"mvex" => self.parse_mvex(child)?,
                _ => {}
            }
            cursor += size;
        }
        Ok(())
    }

    fn parse_mvhd(&mut self, payload: &[u8]) -> MediaResult<()> {
        if payload.len() < 16 {
            return Ok(());
        }
        let version = payload[0];
        if version == 1 {
            if payload.len() >= 28 {
                let timescale = read_u32(&payload[20..24]);
                let duration = read_u64(&payload[24..32]);
                self.movie_timescale = if timescale > 0 { timescale } else { 1000 };
                if duration > 0 && self.movie_timescale > 0 {
                    self.duration_sec = Some(duration as f64 / self.movie_timescale as f64);
                }
            }
        } else {
            let timescale = read_u32(&payload[12..16]);
            let duration = read_u32(&payload[16..20]);
            self.movie_timescale = if timescale > 0 { timescale } else { 1000 };
            if duration > 0 && self.movie_timescale > 0 {
                self.duration_sec = Some(duration as f64 / self.movie_timescale as f64);
            }
        }
        Ok(())
    }

    fn parse_trak(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        let mut track_id = 0u32;
        let mut width = 0u32;
        let mut height = 0u32;
        let mut track_type = TrackType::Video;
        let mut timescale = 1000u64;
        let mut audio_cfg = None;
        let mut video_cfg = None;

        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            match box_type {
                b"tkhd" => {
                    if child.len() >= 20 {
                        let version = child[0];
                        if version == 1 {
                            if child.len() >= 32 {
                                track_id = read_u32(&child[20..24]);
                            }
                        } else {
                            track_id = read_u32(&child[12..16]);
                        }
                        if child.len() >= 80 {
                            width =
                                (read_u32(&child[child.len() - 8..child.len() - 4]) >> 16) & 0xFFFF;
                            height = (read_u32(&child[child.len() - 4..]) >> 16) & 0xFFFF;
                        }
                    }
                }
                b"mdia" => {
                    self.parse_mdia(
                        child,
                        &mut track_type,
                        &mut timescale,
                        &mut audio_cfg,
                        &mut video_cfg,
                    )?;
                }
                _ => {}
            }
            cursor += size;
        }

        if track_id > 0 {
            if track_type == TrackType::Video {
                if let Some(ref mut v) = video_cfg {
                    if v.width == 0 {
                        v.width = width;
                    }
                    if v.height == 0 {
                        v.height = height;
                    }
                } else if width > 0 && height > 0 {
                    video_cfg = Some(VideoConfig {
                        width,
                        height,
                        codec: "unknown".into(),
                        extra_data: Vec::new(),
                    });
                }
            }

            self.tracks.push(MediaTrack {
                id: track_id,
                track_type,
                timescale,
                duration: self.duration_sec,
                audio_config: audio_cfg,
                video_config: video_cfg,
            });
        }

        Ok(())
    }

    fn parse_mdia(
        &self,
        payload: &[u8],
        track_type: &mut TrackType,
        timescale: &mut u64,
        audio_cfg: &mut Option<AudioConfig>,
        video_cfg: &mut Option<VideoConfig>,
    ) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            match box_type {
                b"mdhd" => {
                    if child.len() >= 16 {
                        let version = child[0];
                        if version == 1 {
                            if child.len() >= 28 {
                                *timescale = read_u32(&child[20..24]) as u64;
                            }
                        } else {
                            *timescale = read_u32(&child[12..16]) as u64;
                        }
                    }
                }
                b"hdlr" => {
                    if child.len() >= 12 {
                        let handler = &child[8..12];
                        if handler == b"soun" {
                            *track_type = TrackType::Audio;
                        } else if handler == b"vide" {
                            *track_type = TrackType::Video;
                        }
                    }
                }
                b"minf" => {
                    self.parse_minf(child, *track_type, audio_cfg, video_cfg)?;
                }
                _ => {}
            }
            cursor += size;
        }
        Ok(())
    }

    fn parse_minf(
        &self,
        payload: &[u8],
        track_type: TrackType,
        audio_cfg: &mut Option<AudioConfig>,
        video_cfg: &mut Option<VideoConfig>,
    ) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            if box_type == b"stbl" {
                self.parse_stbl(child, track_type, audio_cfg, video_cfg)?;
            }
            cursor += size;
        }
        Ok(())
    }

    fn parse_stbl(
        &self,
        payload: &[u8],
        track_type: TrackType,
        audio_cfg: &mut Option<AudioConfig>,
        video_cfg: &mut Option<VideoConfig>,
    ) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            if box_type == b"stsd" && child.len() >= 8 {
                let entry_count = read_u32(&child[4..8]);
                let mut entry_cursor = 8;
                for _ in 0..entry_count {
                    if entry_cursor + 8 > child.len() {
                        break;
                    }
                    let entry_size = read_u32(&child[entry_cursor..entry_cursor + 4]) as usize;
                    if entry_size < 8 || entry_cursor + entry_size > child.len() {
                        break;
                    }
                    let codec_fourcc = &child[entry_cursor + 4..entry_cursor + 8];
                    let entry_payload = &child[entry_cursor + 8..entry_cursor + entry_size];
                    let codec_name = String::from_utf8_lossy(codec_fourcc).trim().to_string();

                    match track_type {
                        TrackType::Audio => {
                            let mut sample_rate = 48000;
                            let mut channels = 2;
                            let mut bit_depth = 16;
                            if entry_payload.len() >= 20 {
                                channels = read_u16(&entry_payload[16..18]);
                                bit_depth = read_u16(&entry_payload[18..20]);
                                if entry_payload.len() >= 28 {
                                    let rate_fp = read_u32(&entry_payload[24..28]) >> 16;
                                    if rate_fp > 0 {
                                        sample_rate = rate_fp;
                                    }
                                }
                            }
                            *audio_cfg = Some(AudioConfig {
                                sample_rate,
                                channels,
                                bit_depth,
                                codec: codec_name,
                                extra_data: entry_payload.to_vec(),
                            });
                        }
                        TrackType::Video => {
                            let mut width = 0;
                            let mut height = 0;
                            if entry_payload.len() >= 28 {
                                width = read_u16(&entry_payload[24..26]) as u32;
                                height = read_u16(&entry_payload[26..28]) as u32;
                            }
                            *video_cfg = Some(VideoConfig {
                                width,
                                height,
                                codec: codec_name,
                                extra_data: entry_payload.to_vec(),
                            });
                        }
                    }
                    entry_cursor += entry_size;
                }
            }
            cursor += size;
        }
        Ok(())
    }

    fn parse_mvex(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            if box_type == b"trex" && child.len() >= 24 {
                let track_id = read_u32(&child[4..8]);
                let default_sample_description_index = read_u32(&child[8..12]);
                let default_sample_duration = read_u32(&child[12..16]);
                let default_sample_size = read_u32(&child[16..20]);
                let default_sample_flags = read_u32(&child[20..24]);

                self.track_defaults.insert(
                    track_id,
                    TrackDefaults {
                        _default_sample_description_index: default_sample_description_index,
                        default_sample_duration,
                        default_sample_size,
                        default_sample_flags,
                    },
                );
            }
            cursor += size;
        }
        Ok(())
    }

    fn parse_moof(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];
            if box_type == b"traf" {
                self.parse_traf(child)?;
            }
            cursor += size;
        }
        Ok(())
    }

    fn parse_traf(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        let mut track_id = 0u32;
        let mut base_decode_time = 0u64;
        let mut default_sample_duration = 0u32;
        let mut default_sample_size = 0u32;
        let mut default_sample_flags = 0u32;

        let mut truns: Vec<PendingFragmentRun> = Vec::new();

        while cursor + 8 <= payload.len() {
            let size = read_u32(&payload[cursor..cursor + 4]) as usize;
            let box_type = &payload[cursor + 4..cursor + 8];
            if size < 8 || cursor + size > payload.len() {
                break;
            }
            let child = &payload[cursor + 8..cursor + size];

            match box_type {
                b"tfhd" => {
                    if child.len() >= 8 {
                        let flags = read_u24(&child[1..4]);
                        track_id = read_u32(&child[4..8]);
                        let mut tfhd_cursor = 8;
                        if flags & 0x01 != 0 {
                            tfhd_cursor += 8;
                        }
                        if flags & 0x02 != 0 {
                            tfhd_cursor += 4;
                        }
                        if flags & 0x08 != 0 && tfhd_cursor + 4 <= child.len() {
                            default_sample_duration =
                                read_u32(&child[tfhd_cursor..tfhd_cursor + 4]);
                            tfhd_cursor += 4;
                        }
                        if flags & 0x10 != 0 && tfhd_cursor + 4 <= child.len() {
                            default_sample_size = read_u32(&child[tfhd_cursor..tfhd_cursor + 4]);
                            tfhd_cursor += 4;
                        }
                        if flags & 0x20 != 0 && tfhd_cursor + 4 <= child.len() {
                            default_sample_flags = read_u32(&child[tfhd_cursor..tfhd_cursor + 4]);
                        }
                    }
                }
                b"tfdt" => {
                    if child.len() >= 8 {
                        let version = child[0];
                        if version == 1 && child.len() >= 12 {
                            base_decode_time = read_u64(&child[4..12]);
                        } else {
                            base_decode_time = read_u32(&child[4..8]) as u64;
                        }
                    }
                }
                b"trun" => {
                    if child.len() >= 8 {
                        let version = child[0];
                        let flags = read_u24(&child[1..4]);
                        let sample_count = read_u32(&child[4..8]) as usize;
                        let mut trun_cursor = 8;

                        if flags & 0x01 != 0 {
                            trun_cursor += 4;
                        }

                        let mut first_sample_flags = default_sample_flags;
                        if flags & 0x04 != 0 && trun_cursor + 4 <= child.len() {
                            first_sample_flags = read_u32(&child[trun_cursor..trun_cursor + 4]);
                            trun_cursor += 4;
                        }

                        let mut run_samples = Vec::with_capacity(sample_count);
                        let trex = self
                            .track_defaults
                            .get(&track_id)
                            .cloned()
                            .unwrap_or_default();
                        let dur_fallback = if default_sample_duration > 0 {
                            default_sample_duration
                        } else {
                            trex.default_sample_duration
                        };
                        let size_fallback = if default_sample_size > 0 {
                            default_sample_size
                        } else {
                            trex.default_sample_size
                        };
                        let flags_fallback = if default_sample_flags > 0 {
                            default_sample_flags
                        } else {
                            trex.default_sample_flags
                        };

                        for i in 0..sample_count {
                            let mut sample_dur = dur_fallback;
                            let mut sample_sz = size_fallback;
                            let mut sample_fl = if i == 0 {
                                first_sample_flags
                            } else {
                                flags_fallback
                            };
                            let mut sample_cto = 0i32;

                            if flags & 0x0100 != 0 && trun_cursor + 4 <= child.len() {
                                sample_dur = read_u32(&child[trun_cursor..trun_cursor + 4]);
                                trun_cursor += 4;
                            }
                            if flags & 0x0200 != 0 && trun_cursor + 4 <= child.len() {
                                sample_sz = read_u32(&child[trun_cursor..trun_cursor + 4]);
                                trun_cursor += 4;
                            }
                            if flags & 0x0400 != 0 && trun_cursor + 4 <= child.len() {
                                sample_fl = read_u32(&child[trun_cursor..trun_cursor + 4]);
                                trun_cursor += 4;
                            }
                            if flags & 0x0800 != 0 && trun_cursor + 4 <= child.len() {
                                sample_cto = if version == 0 {
                                    read_u32(&child[trun_cursor..trun_cursor + 4]) as i32
                                } else {
                                    read_i32(&child[trun_cursor..trun_cursor + 4])
                                };
                                trun_cursor += 4;
                            }

                            run_samples.push(ParsedRunSample {
                                duration: sample_dur,
                                size: sample_sz,
                                flags: sample_fl,
                                cto: sample_cto,
                            });
                        }

                        truns.push(PendingFragmentRun {
                            track_id,
                            base_decode_time,
                            samples: run_samples,
                        });
                    }
                }
                _ => {}
            }
            cursor += size;
        }

        self.pending_runs.extend(truns);
        Ok(())
    }

    fn parse_mdat(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut mdat_cursor = 0;

        while !self.pending_runs.is_empty() {
            let run = self.pending_runs.remove(0);
            let timescale = self
                .tracks
                .iter()
                .find(|t| t.id == run.track_id)
                .map(|t| t.timescale)
                .unwrap_or(1000) as f64;

            let mut current_dts = run.base_decode_time;

            for sample_meta in run.samples {
                let sample_size = sample_meta.size as usize;
                if mdat_cursor + sample_size > payload.len() {
                    break;
                }
                let sample_bytes = payload[mdat_cursor..mdat_cursor + sample_size].to_vec();
                mdat_cursor += sample_size;

                let dts_sec = current_dts as f64 / timescale;
                let pts_sec = (current_dts as i64 + sample_meta.cto as i64) as f64 / timescale;
                let duration_sec = sample_meta.duration as f64 / timescale;

                let is_sync = (sample_meta.flags & 0x00010000) == 0;

                self.samples_queue.push_back(MediaSample {
                    track_id: run.track_id,
                    pts: pts_sec,
                    dts: dts_sec,
                    duration: duration_sec,
                    is_sync_point: is_sync,
                    data: sample_bytes,
                });

                self.buffered.push((pts_sec, pts_sec + duration_sec));
                current_dts += sample_meta.duration as u64;
            }
        }

        merge_time_ranges(&mut self.buffered);
        Ok(())
    }
}

impl MediaDemuxer for Fmp4Demuxer {
    fn append_data(&mut self, chunk: &[u8]) -> MediaResult<()> {
        self.buffer.extend_from_slice(chunk);
        self.parse_boxes()
    }

    fn is_init_complete(&self) -> bool {
        self.is_init_done
    }

    fn tracks(&self) -> &[MediaTrack] {
        &self.tracks
    }

    fn duration(&self) -> Option<f64> {
        self.duration_sec
    }

    fn poll_sample(&mut self) -> Option<MediaSample> {
        self.samples_queue.pop_front()
    }

    fn buffered_ranges(&self) -> Vec<(f64, f64)> {
        self.buffered.clone()
    }

    fn seek(&mut self, target_time: f64) -> MediaResult<()> {
        self.samples_queue.retain(|s| s.pts >= target_time);
        Ok(())
    }

    fn remove_range(&mut self, start: f64, end: f64) {
        self.samples_queue.retain(|s| s.pts < start || s.pts >= end);
        let mut new_buffered = Vec::new();
        for &(r_start, r_end) in &self.buffered {
            if r_end <= start || r_start >= end {
                new_buffered.push((r_start, r_end));
            } else {
                if r_start < start {
                    new_buffered.push((r_start, start));
                }
                if r_end > end {
                    new_buffered.push((end, r_end));
                }
            }
        }
        self.buffered = new_buffered;
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.pending_runs.clear();
        self.samples_queue.clear();
    }
}

#[inline]
fn read_u16(buf: &[u8]) -> u16 {
    u16::from_be_bytes([buf[0], buf[1]])
}

#[inline]
fn read_u24(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32)
}

#[inline]
fn read_u32(buf: &[u8]) -> u32 {
    u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])
}

#[inline]
fn read_i32(buf: &[u8]) -> i32 {
    i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])
}

#[inline]
fn read_u64(buf: &[u8]) -> u64 {
    u64::from_be_bytes([
        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
    ])
}
