
#![deny(unsafe_code)]

use super::traits::AudioDecoder;
use crate::error::{MediaError, MediaResult};
use crate::sample::{DecodedAudioFrame, MediaSample};
use symphonia_codec_vorbis::VorbisDecoder;
use symphonia_core::audio::{AudioBufferRef, Signal};
use symphonia_core::codecs::{CodecParameters, Decoder, DecoderOptions, CODEC_TYPE_VORBIS};
use symphonia_core::formats::Packet;

pub struct VorbisAudioDecoder {
    inner: Option<VorbisDecoder>,
    sample_rate: u32,
    channels: u16,
    extra_data: Vec<u8>,
}

impl VorbisAudioDecoder {
    pub fn new(sample_rate: u32, channels: u16, extra_data: Vec<u8>) -> Self {
        let mut decoder = Self {
            inner: None,
            sample_rate: if sample_rate == 0 { 48000 } else { sample_rate },
            channels: if channels == 0 { 2 } else { channels },
            extra_data,
        };
        decoder.init_inner();
        decoder
    }

    fn init_inner(&mut self) {
        if self.extra_data.is_empty() {
            return;
        }
        let mut params = CodecParameters::new();
        params
            .for_codec(CODEC_TYPE_VORBIS)
            .with_sample_rate(self.sample_rate)
            .with_extra_data(self.extra_data.clone().into_boxed_slice());

        let options = DecoderOptions::default();
        if let Ok(dec) = VorbisDecoder::try_new(&params, &options) {
            self.inner = Some(dec);
        }
    }
}

impl AudioDecoder for VorbisAudioDecoder {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Vec<DecodedAudioFrame>> {
        if sample.data.is_empty() {
            return Ok(Vec::new());
        }

        if self.inner.is_none() {
            self.extra_data = sample.data.clone();
            self.init_inner();
            return Ok(Vec::new());
        }

        let decoder = match self.inner.as_mut() {
            Some(d) => d,
            None => {
                return Err(MediaError::DecodeError(
                    "Vorbis",
                    "Decoder not initialized".into(),
                ))
            }
        };

        let packet = Packet::new_from_slice(0, 0, 0, &sample.data);
        let audio_buf = decoder.decode(&packet).map_err(|e| {
            MediaError::DecodeError("Vorbis", format!("Vorbis decode failed: {:?}", e))
        })?;

        let mut samples_f32 = Vec::new();
        match audio_buf {
            AudioBufferRef::F32(buf) => {
                let channels = buf.spec().channels.count();
                let frames = buf.frames();
                samples_f32.reserve(frames * channels);
                for i in 0..frames {
                    for ch in 0..channels {
                        samples_f32.push(buf.chan(ch)[i]);
                    }
                }
            }
            AudioBufferRef::S16(buf) => {
                let channels = buf.spec().channels.count();
                let frames = buf.frames();
                samples_f32.reserve(frames * channels);
                for i in 0..frames {
                    for ch in 0..channels {
                        samples_f32.push(buf.chan(ch)[i] as f32 / 32768.0);
                    }
                }
            }
            _ => {
                return Err(MediaError::DecodeError(
                    "Vorbis",
                    "Unsupported audio buffer sample format".into(),
                ))
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

    fn reset(&mut self) {
        if let Some(ref mut d) = self.inner {
            d.reset();
        }
    }
}
