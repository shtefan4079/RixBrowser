
#![deny(unsafe_code)]

use super::traits::AudioDecoder;
use crate::error::{MediaError, MediaResult};
use crate::sample::{DecodedAudioFrame, MediaSample};
use rusty_aac::AacDecoder;

pub struct AacAudioDecoder {
    inner: AacDecoder,
    sample_rate: u32,
    channels: u16,
}

impl AacAudioDecoder {
    pub fn new(sample_rate: u32, channels: u16, extra_data: &[u8]) -> Self {
        let sr = if sample_rate == 0 { 44100 } else { sample_rate };
        let ch = if channels == 0 { 2 } else { channels };
        let inner = if let Some(cfg_bytes) = extract_audio_specific_config(extra_data) {
            AacDecoder::with_config_bytes(&cfg_bytes).unwrap_or_default()
        } else {
            AacDecoder::new()
        };
        let actual_sr = inner.output_sample_rate().unwrap_or(sr);
        Self {
            inner,
            sample_rate: actual_sr,
            channels: ch,
        }
    }
}

impl AudioDecoder for AacAudioDecoder {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Vec<DecodedAudioFrame>> {
        if sample.data.is_empty() {
            return Ok(Vec::new());
        }
        match self.inner.decode(&sample.data, Some((sample.pts * 1e6) as i64)) {
            Ok(decoded) => {
                let sr = decoded.sample_rate;
                let ch = decoded.channels;
                if sr > 0 {
                    self.sample_rate = sr;
                }
                if ch > 0 {
                    self.channels = ch;
                }
                Ok(vec![DecodedAudioFrame {
                    pts: sample.pts,
                    channels: self.channels,
                    sample_rate: self.sample_rate,
                    samples_f32: decoded.samples,
                }])
            }
            Err(e) => Err(MediaError::DecodeError("AAC", format!("{:?}", e))),
        }
    }

    fn flush(&mut self) -> MediaResult<Vec<DecodedAudioFrame>> {
        Ok(Vec::new())
    }

    fn reset(&mut self) {
        self.inner = AacDecoder::new();
    }
}

fn read_var_size(data: &[u8], pos: &mut usize) -> Option<usize> {
    let mut size: usize = 0;
    for _ in 0..4 {
        if *pos >= data.len() {
            return None;
        }
        let b = data[*pos];
        *pos += 1;
        size = (size << 7) | (b & 0x7F) as usize;
        if b & 0x80 == 0 {
            return Some(size);
        }
    }
    None
}

fn find_box_payload<'a>(data: &'a [u8], box_type: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i + 8 <= data.len() {
        let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        if size < 8 || i + size > data.len() {
            i += 1;
            continue;
        }
        if &data[i + 4..i + 8] == box_type {
            return Some(&data[i + 8..i + size]);
        }
        i += size;
    }
    None
}

fn extract_audio_specific_config(data: &[u8]) -> Option<Vec<u8>> {
    let esds_payload = find_box_payload(data, b"esds").unwrap_or(data);
    if esds_payload.len() < 4 {
        return None;
    }
    find_decoder_specific_info(esds_payload, 4, esds_payload.len())
}

fn find_decoder_specific_info(data: &[u8], start: usize, end: usize) -> Option<Vec<u8>> {
    let mut pos = start;
    while pos + 2 <= end {
        let tag = data[pos];
        let mut cursor = pos + 1;
        let size = read_var_size(data, &mut cursor)?;
        let payload = cursor;
        let next = payload.checked_add(size)?;
        if next > end {
            break;
        }
        match tag {
            0x05 => return Some(data[payload..next].to_vec()),
            0x03 if payload + 3 <= next => {
                if let Some(found) = find_decoder_specific_info(data, payload + 3, next) {
                    return Some(found);
                }
            }
            0x04 if payload + 13 <= next => {
                if let Some(found) = find_decoder_specific_info(data, payload + 13, next) {
                    return Some(found);
                }
            }
            _ => {}
        }
        pos = next;
    }
    None
}
