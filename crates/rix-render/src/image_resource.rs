use std::io::Cursor;
use vello::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

const MAX_RAW_RGBA_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ImageResource {
    pub data: ImageData,
}

impl PartialEq for ImageResource {
    fn eq(&self, other: &Self) -> bool {
        self.data.data.id() == other.data.data.id()
    }
}

impl ImageResource {
    pub fn from_rgba(width: u32, height: u32, pixels: &[u8]) -> Result<Self, String> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|bytes| *bytes <= MAX_RAW_RGBA_BYTES)
            .ok_or_else(|| {
                "RGBA image dimensions overflow or exceed the resource budget".to_owned()
            })?;
        if width == 0 || height == 0 || pixels.len() != expected {
            return Err("RGBA image dimensions do not match its pixel data".into());
        }
        Ok(Self {
            data: ImageData {
                data: Blob::from(pixels.to_vec()),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width,
                height,
            },
        })
    }

    pub fn decode_data_uri(source: &str, max_decoded_bytes: u64) -> Result<Self, String> {
        use base64::Engine;
        let (header, payload) = source.split_once(',').ok_or("Invalid image data URI")?;
        if !header.starts_with("data:image/") {
            return Err("Data URI does not contain an image".into());
        }
        let bytes = if header
            .split(';')
            .any(|part| part.eq_ignore_ascii_case("base64"))
        {
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(|e| e.to_string())?
        } else {
            let mut bytes = Vec::with_capacity(payload.len());
            let mut input = payload.as_bytes().iter().copied();
            while let Some(byte) = input.next() {
                if byte == b'%' {
                    let hi = input
                        .next()
                        .and_then(|byte| (byte as char).to_digit(16))
                        .ok_or("Invalid data URI escape")?;
                    let lo = input
                        .next()
                        .and_then(|byte| (byte as char).to_digit(16))
                        .ok_or("Invalid data URI escape")?;
                    bytes.push(((hi << 4) | lo) as u8);
                } else {
                    bytes.push(byte);
                }
            }
            bytes
        };
        Self::decode(&bytes, max_decoded_bytes)
    }

    pub fn decode(bytes: &[u8], max_decoded_bytes: u64) -> Result<Self, String> {
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(max_decoded_bytes);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|e| e.to_string())?;
        let width = decoded.width();
        let height = decoded.height();
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) * 4 > max_decoded_bytes
        {
            return Err("Image dimensions exceed decoded-byte budget".into());
        }
        Ok(Self {
            data: ImageData {
                data: Blob::from(decoded.to_rgba8().into_raw()),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width,
                height,
            },
        })
    }

    pub fn byte_len(&self) -> usize {
        self.data.data.len()
    }
}
