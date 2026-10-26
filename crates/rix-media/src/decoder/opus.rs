
#![deny(unsafe_code)]

use super::traits::AudioDecoder;
use crate::error::{MediaError, MediaResult};
use crate::sample::{DecodedAudioFrame, MediaSample};
use opus_pure::OpusDecoder as PureOpusDecoder;

pub struct OpusAudioDecoder {
    inner: PureOpusDecoder,
    sample_rate: u32,
    channels: u16,
    scratch: Vec<f32>,
}

impl OpusAudioDecoder {
    pub fn new(sample_rate: u32, channels: u16) -> MediaResult<Self> {
        let rate = if sample_rate == 0 { 48000 } else { sample_rate };
        let ch = if channels == 0 { 2 } else { channels };
        let inner = PureOpusDecoder::new(rate as i32, ch as usize).map_err(|e| {
            MediaError::DecodeError("Opus", format!("Failed to init decoder: {:?}", e))
        })?;

        let max_samples = 5760 * (ch as usize);
        Ok(Self {
            inner,
            sample_rate: rate,
            channels: ch,
            scratch: vec![0.0f32; max_samples],
        })
    }
}

impl AudioDecoder for OpusAudioDecoder {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Vec<DecodedAudioFrame>> {
        if sample.data.is_empty() {
            return Ok(Vec::new());
        }

        let max_frame_size = 5760;
        let decoded_samples_per_ch = self
            .inner
            .decode(&sample.data, max_frame_size, &mut self.scratch)
            .map_err(|e| MediaError::DecodeError("Opus", format!("Decode failed: {:?}", e)))?;

        let total_samples = decoded_samples_per_ch * (self.channels as usize);
        let pcm_data = self.scratch[..total_samples].to_vec();

        Ok(vec![DecodedAudioFrame {
            pts: sample.pts,
            channels: self.channels,
            sample_rate: self.sample_rate,
            samples_f32: pcm_data,
        }])
    }

    fn flush(&mut self) -> MediaResult<Vec<DecodedAudioFrame>> {
        Ok(Vec::new())
    }

    fn reset(&mut self) {
        let _ = self.inner.reset_state();
    }
}
