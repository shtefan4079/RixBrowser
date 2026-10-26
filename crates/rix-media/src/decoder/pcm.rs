
#![deny(unsafe_code)]

use super::traits::AudioDecoder;
use crate::error::MediaResult;
use crate::sample::{DecodedAudioFrame, MediaSample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmFormat {
    U8,
    S16Le,
    S16Be,
    S24Le,
    F32Le,
}

pub struct PcmAudioDecoder {
    sample_rate: u32,
    channels: u16,
    format: PcmFormat,
}

impl PcmAudioDecoder {
    pub fn new(sample_rate: u32, channels: u16, format: PcmFormat) -> Self {
        Self {
            sample_rate: if sample_rate == 0 { 44100 } else { sample_rate },
            channels: if channels == 0 { 2 } else { channels },
            format,
        }
    }
}

impl AudioDecoder for PcmAudioDecoder {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Vec<DecodedAudioFrame>> {
        if sample.data.is_empty() {
            return Ok(Vec::new());
        }

        let mut samples_f32 = Vec::new();
        match self.format {
            PcmFormat::U8 => {
                for &b in &sample.data {
                    samples_f32.push((b as f32 - 128.0) / 128.0);
                }
            }
            PcmFormat::S16Le => {
                for chunk in sample.data.chunks_exact(2) {
                    let val = i16::from_le_bytes([chunk[0], chunk[1]]);
                    samples_f32.push(val as f32 / 32768.0);
                }
            }
            PcmFormat::S16Be => {
                for chunk in sample.data.chunks_exact(2) {
                    let val = i16::from_be_bytes([chunk[0], chunk[1]]);
                    samples_f32.push(val as f32 / 32768.0);
                }
            }
            PcmFormat::S24Le => {
                for chunk in sample.data.chunks_exact(3) {
                    let val = ((chunk[0] as i32)
                        | ((chunk[1] as i32) << 8)
                        | (((chunk[2] as i8) as i32) << 16)) as f32;
                    samples_f32.push(val / 8388608.0);
                }
            }
            PcmFormat::F32Le => {
                for chunk in sample.data.chunks_exact(4) {
                    let val = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    samples_f32.push(val);
                }
            }
        }

        Ok(vec![DecodedAudioFrame {
            pts: sample.pts,
            channels: self.channels,
            sample_rate: self.sample_rate,
            samples_f32,
        }])
    }

    fn flush(&mut self) -> MediaResult<Vec<DecodedAudioFrame>> {
        Ok(Vec::new())
    }

    fn reset(&mut self) {}
}
