#![allow(missing_docs)]

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSegment {
    pub header_id: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UmpPart {
    pub part_type: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaHeader {
    pub header_id: u32,
    pub video_id: String,
    pub itag: i32,
    pub lmt: u64,
    pub xtags: String,
    pub is_init: bool,
    pub segment_num: i32,
    pub duration_ms: i64,
    pub start_ms: i64,
    pub segment_length_bytes: i64,
}

pub const UMP_PART_ONESIE_HEADER: u32 = 10;
pub const UMP_PART_ONESIE_DATA: u32 = 11;
pub const UMP_PART_MEDIA_HEADER: u32 = 20;
pub const UMP_PART_MEDIA: u32 = 21;
pub const UMP_PART_MEDIA_END: u32 = 22;
pub const UMP_PART_NEXT_REQUEST_POLICY: u32 = 35;
pub const UMP_PART_STREAM_PROTECTION_STATUS: u32 = 58;
pub const UMP_PART_FORMAT_SELECTION_CONFIG: u32 = 37;
pub const UMP_PART_SELECTABLE_FORMATS: u32 = 51;

fn encode_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn encode_tag(buf: &mut Vec<u8>, tag: u32) {
    encode_varint(buf, tag as u64);
}

fn encode_varint_field(buf: &mut Vec<u8>, tag: u32, value: u64) {
    encode_tag(buf, tag);
    encode_varint(buf, value);
}

fn encode_bytes_field(buf: &mut Vec<u8>, tag: u32, value: &[u8]) {
    encode_tag(buf, tag);
    encode_varint(buf, value.len() as u64);
    buf.extend_from_slice(value);
}

fn encode_string_field(buf: &mut Vec<u8>, tag: u32, value: &str) {
    encode_bytes_field(buf, tag, value.as_bytes());
}

fn encode_message_field(buf: &mut Vec<u8>, tag: u32, encode: impl FnOnce(&mut Vec<u8>)) {
    let mut tmp = Vec::new();
    encode(&mut tmp);
    encode_tag(buf, tag);
    encode_varint(buf, tmp.len() as u64);
    buf.extend_from_slice(&tmp);
}

fn decode_varint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    let mut i = offset;
    loop {
        if i >= data.len() {
            return None;
        }
        let byte = data[i];
        i += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i - offset));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

fn read_ump_varint(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    if offset >= data.len() {
        return None;
    }
    let first = data[offset];
    let (size, mask) = if first < 0x80 {
        (1usize, 0x7fu8)
    } else if first < 0xc0 {
        (2usize, 0x3fu8)
    } else if first < 0xe0 {
        (3usize, 0x1fu8)
    } else if first < 0xf0 {
        (4usize, 0x0fu8)
    } else if first < 0xf8 {
        (5usize, 0x07u8)
    } else {
        return None;
    };
    if offset + size > data.len() {
        return None;
    }
    let mut value = (first & mask) as u32;
    for i in 1..size {
        value = (value << 8) | (data[offset + i] as u32);
    }
    Some((value, size))
}

pub fn parse_ump(data: &[u8]) -> Vec<UmpPart> {
    let mut parts = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        let (part_type, n) = match read_ump_varint(data, offset) {
            Some(v) => v,
            None => break,
        };
        offset += n;
        let (size, n) = match read_ump_varint(data, offset) {
            Some(v) => v,
            None => break,
        };
        offset += n;
        let size = size as usize;
        if offset + size > data.len() {
            break;
        }
        parts.push(UmpPart {
            part_type,
            data: data[offset..offset + size].to_vec(),
        });
        offset += size;
    }
    parts
}

pub fn parse_ump_response(data: &[u8]) -> Vec<MediaSegment> {
    let mut segments = Vec::new();
    for part in parse_ump(data) {
        if part.part_type != UMP_PART_MEDIA {
            continue;
        }
        if part.data.is_empty() {
            continue;
        }
        segments.push(MediaSegment {
            header_id: part.data[0] as u32,
            data: part.data[1..].to_vec(),
        });
    }
    segments
}

pub fn parse_media_header(data: &[u8]) -> MediaHeader {
    let mut h = MediaHeader::default();
    let mut offset = 0usize;
    while offset < data.len() {
        let (tag, n) = match decode_varint(data, offset) {
            Some(v) => v,
            None => break,
        };
        offset += n;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        match (field, wire) {
            (1, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.header_id = v as u32;
                    offset += n;
                } else {
                    break;
                }
            }
            (2, 2) => {
                if let Some((len, n)) = decode_varint(data, offset) {
                    offset += n;
                    let len = len as usize;
                    if offset + len <= data.len() {
                        h.video_id = String::from_utf8_lossy(&data[offset..offset + len]).to_string();
                        offset += len;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            (3, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.itag = v as i32;
                    offset += n;
                } else {
                    break;
                }
            }
            (4, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.lmt = v;
                    offset += n;
                } else {
                    break;
                }
            }
            (5, 2) => {
                if let Some((len, n)) = decode_varint(data, offset) {
                    offset += n;
                    let len = len as usize;
                    if offset + len <= data.len() {
                        h.xtags = String::from_utf8_lossy(&data[offset..offset + len]).to_string();
                        offset += len;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            (8, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.is_init = v != 0;
                    offset += n;
                } else {
                    break;
                }
            }
            (9, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.segment_num = v as i32;
                    offset += n;
                } else {
                    break;
                }
            }
            (10, 0) => {
                if let Some((_, n)) = decode_varint(data, offset) {
                    offset += n;
                } else {
                    break;
                }
            }
            (11, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.start_ms = v as i64;
                    offset += n;
                } else {
                    break;
                }
            }
            (12, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.duration_ms = v as i64;
                    offset += n;
                } else {
                    break;
                }
            }
            (14, 0) => {
                if let Some((v, n)) = decode_varint(data, offset) {
                    h.segment_length_bytes = v as i64;
                    offset += n;
                } else {
                    break;
                }
            }
            (_, 2) => {
                if let Some((len, n)) = decode_varint(data, offset) {
                    offset += n;
                    let len = len as usize;
                    if offset + len <= data.len() {
                        offset += len;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            (_, 0) => {
                if let Some((_, n)) = decode_varint(data, offset) {
                    offset += n;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    h
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub hl: String,
    pub client_name: i32,
    pub client_version: String,
    pub os_name: String,
    pub os_version: String,
    pub device_make: String,
    pub device_model: String,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            hl: "en".to_string(),
            client_name: 1,
            client_version: "2.20250101.00.00".to_string(),
            os_name: "Windows".to_string(),
            os_version: "10.0".to_string(),
            device_make: String::new(),
            device_model: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SabrRequestParams {
    pub video_id: String,
    pub video_itag: i32,
    pub audio_itag: i32,
    pub client_info: ClientInfo,
    pub streamer_context: Option<StreamerContext>,
    pub buffered_ranges: Vec<BufferedRange>,
    pub media_start_time_ms: i64,
    pub last_video_itag: i32,
    pub last_audio_itag: i32,
    pub init_format_ids: Vec<i32>,
}

#[derive(Debug, Clone, Default)]
pub struct StreamerContext {
    pub client_info: Option<ClientInfo>,
    pub po_token: Vec<u8>,
    pub playback_cookie: Vec<u8>,
    pub sabr_contexts: Vec<(i32, Vec<u8>)>,
    pub unsent_sabr_contexts: Vec<i32>,
    pub field7: String,
}

#[derive(Debug, Clone, Default)]
pub struct BufferedRange {
    pub itag: i32,
    pub last_modified: String,
    pub xtags: String,
    pub start_time_ms: i64,
    pub duration_ms: i64,
    pub start_segment_index: i32,
    pub end_segment_index: i32,
}

fn encode_format_id(buf: &mut Vec<u8>, tag: u32, itag: i32, last_modified: &str, xtags: &str) {
    encode_message_field(buf, tag, |b| {
        if itag != 0 {
            encode_varint_field(b, 8, itag as u64);
        }
        if last_modified != "0" && !last_modified.is_empty() {
            encode_varint_field(b, 16, last_modified.parse().unwrap_or(0));
        }
        if !xtags.is_empty() {
            encode_string_field(b, 26, xtags);
        }
    });
}

fn encode_client_info_msg(buf: &mut Vec<u8>, ci: &ClientInfo) {
    if !ci.hl.is_empty() {
        encode_string_field(buf, 10, &ci.hl);
    }
    if !ci.device_make.is_empty() {
        encode_string_field(buf, 98, &ci.device_make);
    }
    if !ci.device_model.is_empty() {
        encode_string_field(buf, 106, &ci.device_model);
    }
    if ci.client_name != 0 {
        encode_varint_field(buf, 128, ci.client_name as u64);
    }
    if !ci.client_version.is_empty() {
        encode_string_field(buf, 138, &ci.client_version);
    }
    if !ci.os_name.is_empty() {
        encode_string_field(buf, 146, &ci.os_name);
    }
    if !ci.os_version.is_empty() {
        encode_string_field(buf, 154, &ci.os_version);
    }
}

fn encode_streamer_context(buf: &mut Vec<u8>, sc: &StreamerContext) {
    if let Some(ci) = &sc.client_info {
        encode_message_field(buf, 10, |b| encode_client_info_msg(b, ci));
    }
    if !sc.po_token.is_empty() {
        encode_bytes_field(buf, 18, &sc.po_token);
    }
    if !sc.playback_cookie.is_empty() {
        encode_bytes_field(buf, 26, &sc.playback_cookie);
    }
    for (t, v) in &sc.sabr_contexts {
        encode_message_field(buf, 42, |b| {
            encode_varint_field(b, 8, *t as u64);
            encode_bytes_field(b, 18, v);
        });
    }
    if !sc.unsent_sabr_contexts.is_empty() {
        let mut tmp = Vec::new();
        for v in &sc.unsent_sabr_contexts {
            encode_varint(&mut tmp, *v as u64);
        }
        encode_bytes_field(buf, 50, &tmp);
    }
    if !sc.field7.is_empty() {
        encode_string_field(buf, 58, &sc.field7);
    }
}

fn encode_buffered_range(buf: &mut Vec<u8>, br: &BufferedRange) {
    if br.itag != 0 {
        encode_format_id(buf, 10, br.itag, &br.last_modified, &br.xtags);
    }
    if br.start_time_ms != 0 {
        encode_varint_field(buf, 16, br.start_time_ms as u64);
    }
    if br.duration_ms != 0 {
        encode_varint_field(buf, 24, br.duration_ms as u64);
    }
    if br.start_segment_index != 0 {
        encode_varint_field(buf, 32, br.start_segment_index as u64);
    }
    if br.end_segment_index != 0 {
        encode_varint_field(buf, 40, br.end_segment_index as u64);
    }
}

pub fn build_sabr_request(video_id: &str, itag: i32) -> Vec<u8> {
    let params = SabrRequestParams {
        video_id: video_id.to_string(),
        video_itag: itag,
        client_info: ClientInfo::default(),
        ..Default::default()
    };
    build_sabr_request_params(&params)
}

pub fn build_sabr_request_params(params: &SabrRequestParams) -> Vec<u8> {
    let mut buf = Vec::new();
    if params.video_itag != 0 {
        encode_format_id(&mut buf, 138, params.video_itag, "0", "");
    }
    if params.audio_itag != 0 {
        encode_format_id(&mut buf, 130, params.audio_itag, "0", "");
    }
    for itag in &params.init_format_ids {
        encode_format_id(&mut buf, 18, *itag, "0", "");
    }
    for br in &params.buffered_ranges {
        encode_message_field(&mut buf, 26, |b| encode_buffered_range(b, br));
    }
    if params.media_start_time_ms != 0 {
        encode_varint_field(&mut buf, 32, params.media_start_time_ms as u64);
    }
    if params.last_video_itag != 0 {
        encode_varint_field(&mut buf, 176, params.last_video_itag as u64);
    }
    if params.last_audio_itag != 0 {
        encode_varint_field(&mut buf, 184, params.last_audio_itag as u64);
    }
    let mut sc = params.streamer_context.clone().unwrap_or_default();
    if sc.client_info.is_none() {
        sc.client_info = Some(params.client_info.clone());
    }
    encode_message_field(&mut buf, 154, |b| encode_streamer_context(b, &sc));
    buf
}
