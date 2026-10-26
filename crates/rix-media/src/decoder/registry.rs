
#![deny(unsafe_code)]

use super::aac::AacAudioDecoder;
use super::h264::H264VideoDecoder;
use super::opus::OpusAudioDecoder;
use super::pcm::{PcmAudioDecoder, PcmFormat};
use super::traits::{AudioDecoder, VideoDecoder};
use super::vorbis::VorbisAudioDecoder;
use super::vp8::Vp8VideoDecoder;

pub struct CodecRegistry;

impl CodecRegistry {
    pub fn is_container_supported(container_mime: &str) -> bool {
        let clean = container_mime.trim().to_ascii_lowercase();
        matches!(
            clean.as_str(),
            "video/webm"
                | "audio/webm"
                | "video/mp4"
                | "audio/mp4"
                | "application/mp4"
                | "audio/wav"
                | "audio/x-wav"
        )
    }

    fn is_avc1_codec_supported(codec: &str) -> bool {
        if codec == "avc1" || codec == "avc" || codec == "h264" {
            return true;
        }
        let Some(suffix) = codec.strip_prefix("avc1.") else {
            return false;
        };
        if suffix.len() < 2 || !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        matches!(u8::from_str_radix(&suffix[..2], 16), Ok(66 | 77 | 88 | 100))
    }

    fn is_mp4a_codec_supported(codec: &str) -> bool {
        if codec == "mp4a" || codec == "aac" {
            return true;
        }
        let Some(suffix) = codec.strip_prefix("mp4a.") else {
            return false;
        };
        let oti: String = suffix.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        matches!(oti.as_str(), "40" | "66" | "67" | "68")
    }

    pub fn is_codec_supported(container_mime: &str, codec: &str) -> bool {
        let container = container_mime.trim().to_ascii_lowercase();
        let c = codec.trim().to_ascii_lowercase();

        if container.contains("webm") {
            if c == "vp8" || c == "vp8.0" {
                return true;
            }
            if c == "opus" || c.starts_with("opus.") {
                return true;
            }
            if c == "vorbis" || c == "a_vorbis" {
                return true;
            }
            if c == "pcm" || c == "pcm-s16le" {
                return true;
            }
            return false;
        }

        if container.contains("mp4") {
            if Self::is_avc1_codec_supported(&c) {
                return true;
            }
            if Self::is_mp4a_codec_supported(&c) {
                return true;
            }
            if c == "opus" || c == "dops" {
                return true;
            }
            if c == "pcm" || c == "pcm-s16le" || c == "pcm-s16be" || c == "pcm-f32le" {
                return true;
            }
            return false;
        }

        if container.contains("wav") {
            return c.is_empty() || c == "1" || c == "pcm";
        }

        false
    }

    pub fn is_type_supported(full_type: &str) -> bool {
        let trimmed = full_type.trim();
        if trimmed.is_empty() {
            return false;
        }

        let parts: Vec<&str> = trimmed.split(';').map(|s| s.trim()).collect();
        let mime = parts[0].to_ascii_lowercase();

        if !Self::is_container_supported(&mime) {
            return false;
        }

        let mut codecs_found = Vec::new();
        for param in &parts[1..] {
            let lower_param = param.to_ascii_lowercase();
            if let Some(rest) = lower_param.strip_prefix("codecs=") {
                let unquoted = rest.trim_matches('"').trim_matches('\'');
                for c in unquoted.split(',') {
                    let cleaned = c.trim();
                    if !cleaned.is_empty() {
                        codecs_found.push(cleaned.to_string());
                    }
                }
            }
        }

        if codecs_found.is_empty() {
            return true;
        }

        for c in &codecs_found {
            if !Self::is_codec_supported(&mime, c) {
                return false;
            }
        }

        true
    }

    pub fn create_audio_decoder(
        codec: &str,
        sample_rate: u32,
        channels: u16,
        extra_data: &[u8],
    ) -> Option<Box<dyn AudioDecoder>> {
        let lower = codec.trim().to_ascii_lowercase();
        if lower == "opus" || lower.starts_with("opus.") || lower == "dops" || lower == "a_opus" {
            OpusAudioDecoder::new(sample_rate, channels)
                .ok()
                .map(|d| Box::new(d) as Box<dyn AudioDecoder>)
        } else if lower == "vorbis" || lower == "a_vorbis" {
            Some(Box::new(VorbisAudioDecoder::new(
                sample_rate,
                channels,
                Vec::new(),
            )))
        } else if lower == "aac" || lower == "mp4a" || lower.starts_with("mp4a.") {
            Some(Box::new(AacAudioDecoder::new(sample_rate, channels, extra_data)))
        } else if lower.contains("pcm") || lower.contains("wav") || lower == "a_pcm/int/lit" {
            let format = if lower.contains("f32") {
                PcmFormat::F32Le
            } else if lower.contains("s24") {
                PcmFormat::S24Le
            } else if lower.contains("be") {
                PcmFormat::S16Be
            } else if lower.contains("u8") {
                PcmFormat::U8
            } else {
                PcmFormat::S16Le
            };
            Some(Box::new(PcmAudioDecoder::new(
                sample_rate,
                channels,
                format,
            )))
        } else {
            None
        }
    }

    pub fn create_video_decoder(codec: &str, extra_data: &[u8]) -> Option<Box<dyn VideoDecoder>> {
        let lower = codec.trim().to_ascii_lowercase();
        if lower == "vp8" || lower == "vp8.0" || lower == "v_vp8" {
            Some(Box::new(Vp8VideoDecoder::new()))
        } else if lower == "avc1" || lower == "h264" || lower == "avc" || lower.starts_with("avc1.") {
            Some(Box::new(H264VideoDecoder::new(extra_data)))
        } else {
            None
        }
    }
}
