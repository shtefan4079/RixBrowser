
#![deny(unsafe_code)]

use super::traits::VideoDecoder;
use crate::error::{MediaError, MediaResult};
use crate::sample::{DecodedVideoFrame, MediaSample};
use rusty_h264_common::types::YuvFrame;
use rusty_h264_decoder::Decoder as H264Decoder;

pub struct H264VideoDecoder {
    inner: H264Decoder,
    sps_pps_annexb: Vec<u8>,
}

impl H264VideoDecoder {
    pub fn new(extra_data: &[u8]) -> Self {
        let sps_pps = extract_sps_pps(extra_data);
        Self {
            inner: H264Decoder::new(),
            sps_pps_annexb: sps_pps,
        }
    }

    fn avcc_to_annexb(&self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 4);
        if !self.sps_pps_annexb.is_empty() {
            out.extend_from_slice(&self.sps_pps_annexb);
        }
        let mut i = 0;
        while i + 4 <= data.len() {
            let nalu_len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            i += 4;
            if nalu_len == 0 || i + nalu_len > data.len() {
                break;
            }
            out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            out.extend_from_slice(&data[i..i + nalu_len]);
            i += nalu_len;
        }
        if out.len() <= self.sps_pps_annexb.len() + 4 && data.len() > 4 {
            if data.windows(4).any(|w| w == [0x00, 0x00, 0x00, 0x01] || w == [0x00, 0x00, 0x01, 0x00]) {
                let mut o = Vec::with_capacity(self.sps_pps_annexb.len() + data.len());
                if !self.sps_pps_annexb.is_empty() {
                    o.extend_from_slice(&self.sps_pps_annexb);
                }
                o.extend_from_slice(data);
                return o;
            }
        }
        out
    }
}

impl Default for H264VideoDecoder {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl VideoDecoder for H264VideoDecoder {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Option<DecodedVideoFrame>> {
        if sample.data.is_empty() {
            return Ok(None);
        }
        let annexb = self.avcc_to_annexb(&sample.data);
        match self.inner.decode(&annexb) {
            Ok(Some(frame)) => {
                let rgba = yuv420_to_rgba(&frame);
                Ok(Some(DecodedVideoFrame {
                    pts: sample.pts,
                    duration: sample.duration,
                    width: frame.width as u32,
                    height: frame.height as u32,
                    stride: frame.width as u32 * 4,
                    rgba,
                }))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(MediaError::DecodeError("H264", format!("{:?}", e))),
        }
    }

    fn flush(&mut self) -> MediaResult<Option<DecodedVideoFrame>> {
        Ok(None)
    }

    fn reset(&mut self) {
        self.inner = H264Decoder::new();
    }
}

fn extract_sps_pps(data: &[u8]) -> Vec<u8> {
    let avcc = find_box_payload(data, b"avcC").unwrap_or(data);
    if avcc.len() < 7 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let num_sps = (avcc[5] & 0x1F) as usize;
    let mut cursor = 6;
    for _ in 0..num_sps {
        if cursor + 2 > avcc.len() {
            break;
        }
        let len = u16::from_be_bytes([avcc[cursor], avcc[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + len > avcc.len() {
            break;
        }
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(&avcc[cursor..cursor + len]);
        cursor += len;
    }
    if cursor >= avcc.len() {
        return out;
    }
    let num_pps = avcc[cursor] as usize;
    cursor += 1;
    for _ in 0..num_pps {
        if cursor + 2 > avcc.len() {
            break;
        }
        let len = u16::from_be_bytes([avcc[cursor], avcc[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + len > avcc.len() {
            break;
        }
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(&avcc[cursor..cursor + len]);
        cursor += len;
    }
    out
}

fn find_box_payload<'a>(data: &'a [u8], box_type: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i + 8 <= data.len() {
        if &data[i + 4..i + 8] == box_type {
            let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            if size >= 8 && i + size <= data.len() {
                return Some(&data[i + 8..i + size]);
            }
        }
        i += 1;
    }
    None
}

fn yuv420_to_rgba(frame: &YuvFrame) -> Vec<u8> {
    let w = frame.width;
    let h = frame.height;
    let mut rgba = vec![0u8; w * h * 4];
    for j in 0..h {
        for i in 0..w {
            let y = frame.y[j * w + i] as f32;
            let u = frame.u[(j / 2) * (w / 2) + (i / 2)] as f32 - 128.0;
            let v = frame.v[(j / 2) * (w / 2) + (i / 2)] as f32 - 128.0;
            let r = (y + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g = (y - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
            let b = (y + 1.772 * u).clamp(0.0, 255.0) as u8;
            let idx = (j * w + i) * 4;
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = 255;
        }
    }
    rgba
}
