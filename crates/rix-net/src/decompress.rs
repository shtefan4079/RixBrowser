
use crate::error::{NetError, Result};
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use std::io::{Read, Write};

pub const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentEncoding {
    Gzip,
    Deflate,
    Brotli,
    Identity,
}

impl ContentEncoding {
    pub fn from_token(token: &str) -> Self {
        match token.trim().to_lowercase().as_str() {
            "gzip" | "x-gzip" => Self::Gzip,
            "deflate" => Self::Deflate,
            "br" | "brotli" => Self::Brotli,
            _ => Self::Identity,
        }
    }

    pub fn from_header(header_val: &str) -> Self {
        encodings_from_header(header_val)
            .last()
            .copied()
            .unwrap_or(Self::Identity)
    }
}

pub fn encodings_from_header(header_val: &str) -> Vec<ContentEncoding> {
    let trimmed = header_val.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("identity"))
        .map(ContentEncoding::from_token)
        .collect()
}

fn has_unsupported_coding(header_val: &str) -> bool {
    header_val.split(',').any(|t| {
        let token = t.trim().to_lowercase();
        token == "zstd" || token == "compress" || token == "x-compress"
    })
}

pub fn decompress_body(data: &[u8], encoding: ContentEncoding) -> Result<Vec<u8>> {
    decompress_body_limited(data, encoding, MAX_DECOMPRESSED_BYTES)
}

fn decompress_body_limited(
    data: &[u8],
    encoding: ContentEncoding,
    limit: usize,
) -> Result<Vec<u8>> {
    match encoding {
        ContentEncoding::Identity => {
            if data.len() > limit {
                return Err(NetError::ResponseTooLarge(limit));
            }
            Ok(data.to_vec())
        }
        ContentEncoding::Gzip => {
            let mut decoder = GzDecoder::new(data);
            read_capped(&mut decoder, "Gzip", limit)
        }
        ContentEncoding::Deflate => {
            let mut zlib = ZlibDecoder::new(data);
            match read_capped(&mut zlib, "Deflate", limit) {
                Ok(decoded) => return Ok(decoded),
                Err(error @ NetError::ResponseTooLarge(_)) => return Err(error),
                Err(_) => {}
            }
            let mut decoder = DeflateDecoder::new(data);
            read_capped(&mut decoder, "Deflate", limit)
        }
        ContentEncoding::Brotli => {
            let mut reader = brotli::Decompressor::new(data, 4096);
            read_capped(&mut reader, "Brotli", limit)
        }
    }
}

pub fn decompress_body_for_header(data: &[u8], content_encoding: &str) -> Result<Vec<u8>> {
    decompress_body_for_header_limited(data, content_encoding, MAX_DECOMPRESSED_BYTES)
}

pub fn decompress_body_for_header_limited(
    data: &[u8],
    content_encoding: &str,
    limit: usize,
) -> Result<Vec<u8>> {
    if data.len() > limit {
        return Err(NetError::ResponseTooLarge(limit));
    }
    if has_unsupported_coding(content_encoding) {
        return Err(NetError::Decompression(
            "Unsupported Content-Encoding (zstd/compress require C libraries)".to_string(),
        ));
    }
    let mut buf = data.to_vec();
    for encoding in encodings_from_header(content_encoding).iter().rev() {
        buf = decompress_body_limited(&buf, *encoding, limit)?;
    }
    Ok(buf)
}

fn read_capped(reader: &mut dyn Read, label: &str, limit: usize) -> Result<Vec<u8>> {
    let mut decompressed = Vec::new();
    let mut chunk = [0u8; 32 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| NetError::Decompression(format!("{label} decompression failed: {e}")))?;
        if n == 0 {
            break;
        }
        if decompressed.len() + n > limit {
            return Err(NetError::ResponseTooLarge(limit));
        }
        decompressed.extend_from_slice(&chunk[..n]);
    }
    Ok(decompressed)
}

pub struct StreamingDecoder {
    layers: Vec<StreamingLayer>,
    limit: Option<usize>,
    decoded: usize,
}

enum StreamingLayer {
    Gzip(Box<flate2::write::GzDecoder<Vec<u8>>>),
    Deflate {
        decoder: FlateDeflate,
        started: bool,
    },
    Brotli(Box<brotli::DecompressorWriter<Vec<u8>>>),
}

enum FlateDeflate {
    Zlib(Box<flate2::write::ZlibDecoder<Vec<u8>>>),
    Raw(Box<flate2::write::DeflateDecoder<Vec<u8>>>),
}

impl StreamingLayer {
    fn push(&mut self, input: &[u8]) -> Result<()> {
        match self {
            Self::Gzip(decoder) => decoder.write_all(input).map_err(|error| {
                NetError::Decompression(format!("Gzip decompression failed: {error}"))
            }),
            Self::Deflate { decoder, started } => {
                if !*started {
                    let zlib_wrapped = input.first().copied() == Some(0x78);
                    *decoder = if zlib_wrapped {
                        FlateDeflate::Zlib(Box::new(flate2::write::ZlibDecoder::new(Vec::new())))
                    } else {
                        FlateDeflate::Raw(Box::new(flate2::write::DeflateDecoder::new(Vec::new())))
                    };
                    *started = true;
                }
                match decoder {
                    FlateDeflate::Zlib(decoder) => decoder.write_all(input).map_err(|error| {
                        NetError::Decompression(format!("Deflate decompression failed: {error}"))
                    }),
                    FlateDeflate::Raw(decoder) => decoder.write_all(input).map_err(|error| {
                        NetError::Decompression(format!("Deflate decompression failed: {error}"))
                    }),
                }
            }
            Self::Brotli(decoder) => decoder.write_all(input).map_err(|error| {
                NetError::Decompression(format!("Brotli decompression failed: {error}"))
            }),
        }
    }

    fn drain_into(&mut self, out: &mut Vec<u8>) {
        let buffer = match self {
            Self::Gzip(decoder) => decoder.get_mut(),
            Self::Deflate { decoder, .. } => match decoder {
                FlateDeflate::Zlib(decoder) => decoder.get_mut(),
                FlateDeflate::Raw(decoder) => decoder.get_mut(),
            },
            Self::Brotli(decoder) => decoder.get_mut(),
        };
        out.extend_from_slice(buffer);
        buffer.clear();
    }

    fn finish(&mut self) -> Result<()> {
        let result = match self {
            Self::Gzip(decoder) => decoder
                .try_finish()
                .map_err(|error| NetError::Decompression(format!("Gzip body truncated: {error}"))),
            Self::Deflate { decoder, .. } => match decoder {
                FlateDeflate::Zlib(decoder) => decoder.try_finish().map_err(|error| {
                    NetError::Decompression(format!("Deflate body truncated: {error}"))
                }),
                FlateDeflate::Raw(decoder) => decoder.try_finish().map_err(|error| {
                    NetError::Decompression(format!("Deflate body truncated: {error}"))
                }),
            },
            Self::Brotli(decoder) => decoder.flush().map_err(|error| {
                NetError::Decompression(format!("Brotli body truncated: {error}"))
            }),
        };
        result
    }
}

impl StreamingDecoder {
    pub fn for_header(content_encoding: &str, limit: Option<usize>) -> Result<Self> {
        if has_unsupported_coding(content_encoding) {
            return Err(NetError::Decompression(
                "Unsupported Content-Encoding (zstd/compress require C libraries)".to_string(),
            ));
        }
        let mut layers = Vec::new();
        for encoding in encodings_from_header(content_encoding).iter().rev() {
            match encoding {
                ContentEncoding::Identity => {}
                ContentEncoding::Gzip => layers.push(StreamingLayer::Gzip(Box::new(
                    flate2::write::GzDecoder::new(Vec::new()),
                ))),
                ContentEncoding::Deflate => layers.push(StreamingLayer::Deflate {
                    decoder: FlateDeflate::Zlib(Box::new(flate2::write::ZlibDecoder::new(
                        Vec::new(),
                    ))),
                    started: false,
                }),
                ContentEncoding::Brotli => layers.push(StreamingLayer::Brotli(Box::new(
                    brotli::DecompressorWriter::new(Vec::new(), 4096),
                ))),
            }
        }
        let is_identity = layers.is_empty();
        Ok(Self {
            layers,
            limit: if is_identity { None } else { limit },
            decoded: 0,
        })
    }

    pub fn push(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        match self.layers.len() {
            0 => out.extend_from_slice(input),
            1 => {
                self.layers[0].push(input)?;
                self.layers[0].drain_into(out);
            }
            _ => {
                let mut current = input.to_vec();
                for index in 0..self.layers.len() {
                    self.layers[index].push(&current)?;
                    current.clear();
                    self.layers[index].drain_into(&mut current);
                    if current.is_empty() {
                        break;
                    }
                }
                out.append(&mut current);
            }
        }
        self.account(out.len())
    }

    pub fn finish(&mut self, out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        let layer_count = self.layers.len();
        for index in 0..layer_count {
            self.layers[index].finish()?;
            if index + 1 == layer_count {
                self.layers[index].drain_into(out);
                break;
            }
            let mut current = Vec::new();
            self.layers[index].drain_into(&mut current);
            for inner in (index + 1)..layer_count {
                self.layers[inner].push(&current)?;
                current.clear();
                self.layers[inner].drain_into(&mut current);
                if current.is_empty() {
                    break;
                }
            }
            out.append(&mut current);
        }
        self.account(out.len())
    }

    fn account(&mut self, decoded: usize) -> Result<()> {
        self.decoded = self.decoded.saturating_add(decoded);
        if let Some(limit) = self.limit {
            if self.decoded > limit {
                return Err(NetError::ResponseTooLarge(limit));
            }
        }
        Ok(())
    }
}
