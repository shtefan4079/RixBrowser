
#![deny(unsafe_code)]

use super::traits::VideoDecoder;
use crate::error::{MediaError, MediaResult};
use crate::sample::{DecodedVideoFrame, MediaSample};
use image_webp::vp8::Vp8Decoder;
use std::io::Cursor;

pub struct Vp8VideoDecoder {
    last_width: u32,
    last_height: u32,
}

impl Vp8VideoDecoder {
    pub fn new() -> Self {
        Self {
            last_width: 0,
            last_height: 0,
        }
    }
}

impl Default for Vp8VideoDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoDecoder for Vp8VideoDecoder {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Option<DecodedVideoFrame>> {
        if sample.data.is_empty() {
            return Ok(None);
        }

        let is_keyframe = (sample.data[0] & 0x01) == 0;
        if !is_keyframe {
            return Ok(None);
        }

        let cursor = Cursor::new(&sample.data);
        let frame = Vp8Decoder::decode_frame(cursor).map_err(|e| {
            MediaError::DecodeError("VP8", format!("Failed to decode VP8 frame: {:?}", e))
        })?;

        let width = frame.width as usize;
        let height = frame.height as usize;
        self.last_width = width as u32;
        self.last_height = height as u32;

        let uv_width = (width + 1) / 2;
        let mut rgba = vec![0u8; width * height * 4];

        for y in 0..height {
            let y_row = y * width;
            let uv_row = (y / 2) * uv_width;
            let out_row = y * width * 4;

            for x in 0..width {
                let y_idx = y_row + x;
                let uv_idx = uv_row + (x / 2);

                let y_val = frame.ybuf.get(y_idx).copied().unwrap_or(0) as f32;
                let u_val = frame.ubuf.get(uv_idx).copied().unwrap_or(128) as f32 - 128.0;
                let v_val = frame.vbuf.get(uv_idx).copied().unwrap_or(128) as f32 - 128.0;

                let r = (y_val + 1.402 * v_val).clamp(0.0, 255.0) as u8;
                let g = (y_val - 0.344136 * u_val - 0.714136 * v_val).clamp(0.0, 255.0) as u8;
                let b = (y_val + 1.772 * u_val).clamp(0.0, 255.0) as u8;

                let px_idx = out_row + (x * 4);
                rgba[px_idx] = r;
                rgba[px_idx + 1] = g;
                rgba[px_idx + 2] = b;
                rgba[px_idx + 3] = 255;
            }
        }

        Ok(Some(DecodedVideoFrame {
            pts: sample.pts,
            duration: sample.duration,
            width: width as u32,
            height: height as u32,
            stride: (width * 4) as u32,
            rgba,
        }))
    }

    fn flush(&mut self) -> MediaResult<Option<DecodedVideoFrame>> {
        Ok(None)
    }

    fn reset(&mut self) {}
}
