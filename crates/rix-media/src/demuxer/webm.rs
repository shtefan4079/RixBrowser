
#![deny(unsafe_code)]

use super::traits::{merge_time_ranges, MediaDemuxer};
use crate::error::MediaResult;
use crate::sample::{AudioConfig, MediaSample, MediaTrack, TrackType, VideoConfig};
use std::collections::VecDeque;

const ID_EBML_HEADER: u32 = 0x1A45DFA3;
const _ID_DOCTYPE: u32 = 0x4282;

const ID_SEGMENT: u32 = 0x18538067;
const ID_INFO: u32 = 0x1549A966;
const ID_TIMECODE_SCALE: u32 = 0x2AD7B1;
const ID_DURATION: u32 = 0x4489;

const ID_TRACKS: u32 = 0x1654AE6B;
const ID_TRACK_ENTRY: u32 = 0xAE;
const ID_TRACK_NUMBER: u32 = 0xD7;
const ID_TRACK_TYPE: u32 = 0x83;
const ID_CODEC_ID: u32 = 0x86;
const ID_CODEC_PRIVATE: u32 = 0x63A2;
const ID_VIDEO_SETTINGS: u32 = 0xE0;
const ID_PIXEL_WIDTH: u32 = 0xB0;
const ID_PIXEL_HEIGHT: u32 = 0xBA;
const ID_AUDIO_SETTINGS: u32 = 0xE1;
const ID_SAMPLING_FREQ: u32 = 0xB5;
const ID_CHANNELS: u32 = 0x9F;
const ID_BIT_DEPTH: u32 = 0x6264;

const ID_CLUSTER: u32 = 0x1F43B675;
const ID_CLUSTER_TIMECODE: u32 = 0xE7;
const ID_SIMPLE_BLOCK: u32 = 0xA3;
const ID_BLOCK_GROUP: u32 = 0xA0;
const ID_BLOCK: u32 = 0xA1;
const ID_BLOCK_DURATION: u32 = 0x9B;
const ID_REFERENCE_BLOCK: u32 = 0xFB;

pub struct WebmDemuxer {
    buffer: Vec<u8>,
    tracks: Vec<MediaTrack>,
    timecode_scale_ns: u64,
    duration_sec: Option<f64>,
    is_init_done: bool,
    current_cluster_timecode: u64,
    samples_queue: VecDeque<MediaSample>,
    buffered: Vec<(f64, f64)>,
}

impl WebmDemuxer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(64 * 1024),
            tracks: Vec::new(),
            timecode_scale_ns: 1_000_000,
            duration_sec: None,
            is_init_done: false,
            current_cluster_timecode: 0,
            samples_queue: VecDeque::new(),
            buffered: Vec::new(),
        }
    }

    fn parse_stream(&mut self) -> MediaResult<()> {
        let mut cursor = 0;

        while cursor < self.buffer.len() {
            let (id, id_len) = match read_ebml_id(&self.buffer[cursor..]) {
                Some(v) => v,
                None => break,
            };

            let (size_val, size_len) = match read_ebml_size(&self.buffer[cursor + id_len..]) {
                Some(v) => v,
                None => break,
            };

            let header_len = id_len + size_len;

            if id == ID_SEGMENT {
                cursor += header_len;
                continue;
            }

            let size = match size_val {
                Some(s) => s as usize,
                None => {
                    if id == ID_CLUSTER {
                        cursor += header_len;
                        continue;
                    }
                    break;
                }
            };

            if cursor + header_len + size > self.buffer.len() {
                break;
            }

            let payload_start = cursor + header_len;
            let payload_end = payload_start + size;
            let payload = self.buffer[payload_start..payload_end].to_vec();

            match id {
                ID_EBML_HEADER => {
                }
                ID_INFO => {
                    self.parse_info(&payload)?;
                }
                ID_TRACKS => {
                    self.parse_tracks(&payload)?;
                    self.is_init_done = true;
                }
                ID_CLUSTER => {
                    self.parse_cluster(&payload)?;
                }
                ID_CLUSTER_TIMECODE => {
                    self.current_cluster_timecode = read_uint(&payload);
                }
                ID_SIMPLE_BLOCK => {
                    self.parse_simple_block(&payload)?;
                }
                ID_BLOCK_GROUP => {
                    self.parse_block_group(&payload)?;
                }
                _ => {
                }
            }

            cursor = payload_end;
        }

        if cursor > 0 {
            self.buffer.drain(..cursor);
        }

        Ok(())
    }

    fn parse_info(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor < payload.len() {
            let (id, id_len) = match read_ebml_id(&payload[cursor..]) {
                Some(v) => v,
                None => break,
            };
            let (size_opt, size_len) = match read_ebml_size(&payload[cursor + id_len..]) {
                Some(v) => v,
                None => break,
            };
            let size = match size_opt {
                Some(s) => s as usize,
                None => break,
            };
            let start = cursor + id_len + size_len;
            let end = start + size;
            if end > payload.len() {
                break;
            }
            let child = &payload[start..end];

            match id {
                ID_TIMECODE_SCALE => {
                    let scale = read_uint(child);
                    if scale > 0 {
                        self.timecode_scale_ns = scale;
                    }
                }
                ID_DURATION => {
                    let dur_val = read_float(child);
                    if dur_val > 0.0 {
                        self.duration_sec = Some(dur_val * (self.timecode_scale_ns as f64) / 1e9);
                    }
                }
                _ => {}
            }
            cursor = end;
        }
        Ok(())
    }

    fn parse_tracks(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor < payload.len() {
            let (id, id_len) = match read_ebml_id(&payload[cursor..]) {
                Some(v) => v,
                None => break,
            };
            let (size_opt, size_len) = match read_ebml_size(&payload[cursor + id_len..]) {
                Some(v) => v,
                None => break,
            };
            let size = match size_opt {
                Some(s) => s as usize,
                None => break,
            };
            let start = cursor + id_len + size_len;
            let end = start + size;
            if end > payload.len() {
                break;
            }
            let child = &payload[start..end];

            if id == ID_TRACK_ENTRY {
                self.parse_track_entry(child)?;
            }
            cursor = end;
        }
        Ok(())
    }

    fn parse_track_entry(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        let mut track_number = 0u64;
        let mut track_type_code = 0u64;
        let mut codec_id = String::new();
        let mut codec_private = Vec::new();
        let mut width = 0u32;
        let mut height = 0u32;
        let mut sample_rate = 48000u32;
        let mut channels = 2u16;
        let mut bit_depth = 16u16;

        while cursor < payload.len() {
            let (id, id_len) = match read_ebml_id(&payload[cursor..]) {
                Some(v) => v,
                None => break,
            };
            let (size_opt, size_len) = match read_ebml_size(&payload[cursor + id_len..]) {
                Some(v) => v,
                None => break,
            };
            let size = match size_opt {
                Some(s) => s as usize,
                None => break,
            };
            let start = cursor + id_len + size_len;
            let end = start + size;
            if end > payload.len() {
                break;
            }
            let child = &payload[start..end];

            match id {
                ID_TRACK_NUMBER => track_number = read_uint(child),
                ID_TRACK_TYPE => track_type_code = read_uint(child),
                ID_CODEC_ID => codec_id = String::from_utf8_lossy(child).to_string(),
                ID_CODEC_PRIVATE => codec_private = child.to_vec(),
                ID_VIDEO_SETTINGS => {
                    let mut v_cur = 0;
                    while v_cur < child.len() {
                        if let (Some((v_id, v_id_len)), Some((Some(v_sz), v_sz_len))) = (
                            read_ebml_id(&child[v_cur..]),
                            read_ebml_size(&child[v_cur + 1..]),
                        ) {
                            let v_start = v_cur + v_id_len + v_sz_len;
                            let v_end = v_start + v_sz as usize;
                            if v_end <= child.len() {
                                let v_field = &child[v_start..v_end];
                                match v_id {
                                    ID_PIXEL_WIDTH => width = read_uint(v_field) as u32,
                                    ID_PIXEL_HEIGHT => height = read_uint(v_field) as u32,
                                    _ => {}
                                }
                            }
                            v_cur = v_end;
                        } else {
                            break;
                        }
                    }
                }
                ID_AUDIO_SETTINGS => {
                    let mut a_cur = 0;
                    while a_cur < child.len() {
                        if let (Some((a_id, a_id_len)), Some((Some(a_sz), a_sz_len))) = (
                            read_ebml_id(&child[a_cur..]),
                            read_ebml_size(&child[a_cur + 1..]),
                        ) {
                            let a_start = a_cur + a_id_len + a_sz_len;
                            let a_end = a_start + a_sz as usize;
                            if a_end <= child.len() {
                                let a_field = &child[a_start..a_end];
                                match a_id {
                                    ID_SAMPLING_FREQ => sample_rate = read_float(a_field) as u32,
                                    ID_CHANNELS => channels = read_uint(a_field) as u16,
                                    ID_BIT_DEPTH => bit_depth = read_uint(a_field) as u16,
                                    _ => {}
                                }
                            }
                            a_cur = a_end;
                        } else {
                            break;
                        }
                    }
                }
                _ => {}
            }
            cursor = end;
        }

        if track_number > 0 {
            let is_video = track_type_code == 1;
            let track_type = if is_video {
                TrackType::Video
            } else {
                TrackType::Audio
            };

            let audio_config = if !is_video {
                Some(AudioConfig {
                    sample_rate,
                    channels,
                    bit_depth,
                    codec: codec_id.clone(),
                    extra_data: codec_private.clone(),
                })
            } else {
                None
            };

            let video_config = if is_video {
                Some(VideoConfig {
                    width,
                    height,
                    codec: codec_id.clone(),
                    extra_data: codec_private,
                })
            } else {
                None
            };

            self.tracks.push(MediaTrack {
                id: track_number as u32,
                track_type,
                timescale: 1_000_000_000 / self.timecode_scale_ns.max(1),
                duration: self.duration_sec,
                audio_config,
                video_config,
            });
        }

        Ok(())
    }

    fn parse_cluster(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        while cursor < payload.len() {
            let (id, id_len) = match read_ebml_id(&payload[cursor..]) {
                Some(v) => v,
                None => break,
            };
            let (size_opt, size_len) = match read_ebml_size(&payload[cursor + id_len..]) {
                Some(v) => v,
                None => break,
            };
            let size = match size_opt {
                Some(s) => s as usize,
                None => break,
            };
            let start = cursor + id_len + size_len;
            let end = start + size;
            if end > payload.len() {
                break;
            }
            let child = &payload[start..end];

            match id {
                ID_CLUSTER_TIMECODE => {
                    self.current_cluster_timecode = read_uint(child);
                }
                ID_SIMPLE_BLOCK => {
                    self.parse_simple_block(child)?;
                }
                ID_BLOCK_GROUP => {
                    self.parse_block_group(child)?;
                }
                _ => {}
            }
            cursor = end;
        }
        Ok(())
    }

    fn parse_simple_block(&mut self, payload: &[u8]) -> MediaResult<()> {
        if payload.len() < 4 {
            return Ok(());
        }

        let (track_num, track_len) = match read_ebml_size(payload) {
            Some((Some(num), len)) => (num as u32, len),
            _ => return Ok(()),
        };

        if track_len + 3 > payload.len() {
            return Ok(());
        }

        let rel_timecode = i16::from_be_bytes([payload[track_len], payload[track_len + 1]]);
        let flags = payload[track_len + 2];
        let is_keyframe = (flags & 0x80) != 0;
        let lacing = (flags & 0x06) >> 1;

        let data_start = track_len + 3;
        let data_bytes = &payload[data_start..];

        let abs_timecode_ns = ((self.current_cluster_timecode as i64 + rel_timecode as i64).max(0)
            as u64)
            * self.timecode_scale_ns;
        let pts_sec = abs_timecode_ns as f64 / 1e9;

        let default_duration = 0.020;

        let frames = parse_lacing(lacing, data_bytes);
        for frame in frames {
            self.samples_queue.push_back(MediaSample {
                track_id: track_num,
                pts: pts_sec,
                dts: pts_sec,
                duration: default_duration,
                is_sync_point: is_keyframe,
                data: frame,
            });
            self.buffered.push((pts_sec, pts_sec + default_duration));
        }

        merge_time_ranges(&mut self.buffered);
        Ok(())
    }

    fn parse_block_group(&mut self, payload: &[u8]) -> MediaResult<()> {
        let mut cursor = 0;
        let mut block_data = None;
        let mut is_keyframe = true;
        let mut block_duration_ns = None;

        while cursor < payload.len() {
            let (id, id_len) = match read_ebml_id(&payload[cursor..]) {
                Some(v) => v,
                None => break,
            };
            let (size_opt, size_len) = match read_ebml_size(&payload[cursor + id_len..]) {
                Some(v) => v,
                None => break,
            };
            let size = match size_opt {
                Some(s) => s as usize,
                None => break,
            };
            let start = cursor + id_len + size_len;
            let end = start + size;
            if end > payload.len() {
                break;
            }
            let child = &payload[start..end];

            match id {
                ID_BLOCK => block_data = Some(child),
                ID_REFERENCE_BLOCK => is_keyframe = false,
                ID_BLOCK_DURATION => {
                    let dur_tc = read_uint(child);
                    block_duration_ns = Some(dur_tc * self.timecode_scale_ns);
                }
                _ => {}
            }
            cursor = end;
        }

        if let Some(blk) = block_data {
            if blk.len() >= 4 {
                let (track_num, track_len) = match read_ebml_size(blk) {
                    Some((Some(num), len)) => (num as u32, len),
                    _ => return Ok(()),
                };
                if track_len + 3 <= blk.len() {
                    let rel_timecode = i16::from_be_bytes([blk[track_len], blk[track_len + 1]]);
                    let flags = blk[track_len + 2];
                    let lacing = (flags & 0x06) >> 1;
                    let data_bytes = &blk[track_len + 3..];

                    let abs_timecode_ns = ((self.current_cluster_timecode as i64
                        + rel_timecode as i64)
                        .max(0) as u64)
                        * self.timecode_scale_ns;
                    let pts_sec = abs_timecode_ns as f64 / 1e9;
                    let duration_sec = block_duration_ns.map(|ns| ns as f64 / 1e9).unwrap_or(0.020);

                    let frames = parse_lacing(lacing, data_bytes);
                    for frame in frames {
                        self.samples_queue.push_back(MediaSample {
                            track_id: track_num,
                            pts: pts_sec,
                            dts: pts_sec,
                            duration: duration_sec,
                            is_sync_point: is_keyframe,
                            data: frame,
                        });
                        self.buffered.push((pts_sec, pts_sec + duration_sec));
                    }
                    merge_time_ranges(&mut self.buffered);
                }
            }
        }

        Ok(())
    }
}

impl MediaDemuxer for WebmDemuxer {
    fn append_data(&mut self, chunk: &[u8]) -> MediaResult<()> {
        self.buffer.extend_from_slice(chunk);
        self.parse_stream()
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
        self.samples_queue.clear();
        self.current_cluster_timecode = 0;
    }
}

fn parse_lacing(lacing: u8, data: &[u8]) -> Vec<Vec<u8>> {
    if data.is_empty() {
        return Vec::new();
    }
    match lacing {
        0 => {
            vec![data.to_vec()]
        }
        3 => {
            if data.len() < 2 {
                return vec![data.to_vec()];
            }
            let frame_count = (data[0] as usize) + 1;
            let remaining = &data[1..];
            if frame_count == 0 || remaining.len() % frame_count != 0 {
                return vec![data.to_vec()];
            }
            let frame_len = remaining.len() / frame_count;
            remaining.chunks(frame_len).map(|c| c.to_vec()).collect()
        }
        _ => {
            vec![data.to_vec()]
        }
    }
}

fn read_ebml_id(buf: &[u8]) -> Option<(u32, usize)> {
    if buf.is_empty() {
        return None;
    }
    let lead = buf[0];
    let num_bytes = lead.leading_zeros() as usize + 1;
    if num_bytes > 4 || num_bytes > buf.len() {
        return None;
    }
    let mut id = 0u32;
    for &b in &buf[..num_bytes] {
        id = (id << 8) | (b as u32);
    }
    Some((id, num_bytes))
}

fn read_ebml_size(buf: &[u8]) -> Option<(Option<u64>, usize)> {
    if buf.is_empty() {
        return None;
    }
    let lead = buf[0];
    let num_bytes = lead.leading_zeros() as usize + 1;
    if num_bytes > 8 || num_bytes > buf.len() {
        return None;
    }
    let mask = 0xFFu16 >> num_bytes;
    let mut val = (lead as u16 & mask) as u64;
    for &b in &buf[1..num_bytes] {
        val = (val << 8) | (b as u64);
    }
    let unknown_marker = (1u64 << (7 * num_bytes)) - 1;
    if val == unknown_marker {
        Some((None, num_bytes))
    } else {
        Some((Some(val), num_bytes))
    }
}

fn read_uint(buf: &[u8]) -> u64 {
    let mut val = 0u64;
    for &b in buf.iter().take(8) {
        val = (val << 8) | (b as u64);
    }
    val
}

fn read_float(buf: &[u8]) -> f64 {
    if buf.len() == 4 {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(buf);
        f32::from_be_bytes(bytes) as f64
    } else if buf.len() == 8 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(buf);
        f64::from_be_bytes(bytes)
    } else {
        0.0
    }
}
