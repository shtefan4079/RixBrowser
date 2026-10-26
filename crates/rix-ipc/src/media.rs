
#![deny(unsafe_code)]

use crate::protocol::{IpcError, Result, MAX_MESSAGE_SIZE};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MediaSessionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceBufferId(pub u32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaWorkerCommand {
    OpenMediaSession {
        session_id: MediaSessionId,
        generation_id: u64,
    },
    CloseMediaSession {
        session_id: MediaSessionId,
    },
    AddSourceBuffer {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
        mime_type: String,
    },
    RemoveSourceBuffer {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
    },
    AppendSegment {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
        append_id: u64,
        data: Vec<u8>,
    },
    AbortAppend {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
    },
    EndOfStream {
        session_id: MediaSessionId,
        error: Option<String>,
    },
    Seek {
        session_id: MediaSessionId,
        target_time: f64,
    },
    ResetGeneration {
        session_id: MediaSessionId,
        generation_id: u64,
    },
    Play {
        session_id: MediaSessionId,
        playback_rate: f64,
    },
    Pause {
        session_id: MediaSessionId,
    },
    SetPlaybackRate {
        session_id: MediaSessionId,
        playback_rate: f64,
    },
    SetVolume {
        session_id: MediaSessionId,
        volume: f32,
        muted: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaWorkerEvent {
    SessionOpened {
        session_id: MediaSessionId,
    },
    SourceBufferAdded {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
    },
    AppendCompleted {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
        append_id: u64,
        buffered_ranges: Vec<(f64, f64)>,
        duration: Option<f64>,
    },
    AppendAborted {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
    },
    MediaProgress {
        session_id: MediaSessionId,
        buffer_id: SourceBufferId,
        buffered_ranges: Vec<(f64, f64)>,
    },
    MediaError {
        session_id: MediaSessionId,
        buffer_id: Option<SourceBufferId>,
        code: u16,
        message: String,
    },
    VideoFrameReady {
        session_id: MediaSessionId,
        pts: f64,
        duration: f64,
        width: u32,
        height: u32,
        stride: u32,
        buffer_index: Option<u8>,
    },
    AudioSamplesReady {
        session_id: MediaSessionId,
        pts: f64,
        sample_rate: u32,
        channels: u16,
        samples_count: usize,
    },
    SeekCompleted {
        session_id: MediaSessionId,
        current_time: f64,
    },
    PlaybackPositionUpdate {
        session_id: MediaSessionId,
        current_time: f64,
    },
    PlaybackEnded {
        session_id: MediaSessionId,
    },
}

pub fn encode_media_command(cmd: &MediaWorkerCommand) -> Result<Vec<u8>> {
    let payload = postcard::to_allocvec(cmd)?;
    let length = payload.len();
    if length > MAX_MESSAGE_SIZE {
        return Err(IpcError::MessageTooLarge(length, MAX_MESSAGE_SIZE));
    }
    let mut buffer = Vec::with_capacity(4 + length);
    buffer.extend_from_slice(&(length as u32).to_be_bytes());
    buffer.extend_from_slice(&payload);
    Ok(buffer)
}

pub fn decode_media_command(payload: &[u8]) -> Result<MediaWorkerCommand> {
    let cmd = postcard::from_bytes(payload)?;
    Ok(cmd)
}

pub fn encode_media_event(event: &MediaWorkerEvent) -> Result<Vec<u8>> {
    let payload = postcard::to_allocvec(event)?;
    let length = payload.len();
    if length > MAX_MESSAGE_SIZE {
        return Err(IpcError::MessageTooLarge(length, MAX_MESSAGE_SIZE));
    }
    let mut buffer = Vec::with_capacity(4 + length);
    buffer.extend_from_slice(&(length as u32).to_be_bytes());
    buffer.extend_from_slice(&payload);
    Ok(buffer)
}

pub fn decode_media_event(payload: &[u8]) -> Result<MediaWorkerEvent> {
    let event = postcard::from_bytes(payload)?;
    Ok(event)
}

pub struct MediaCommandReader<R> {
    reader: R,
}

impl<R: AsyncRead + Unpin> MediaCommandReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub async fn read_command(&mut self) -> Result<MediaWorkerCommand> {
        let mut len_bytes = [0u8; 4];
        match self.reader.read_exact(&mut len_bytes).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(IpcError::ConnectionClosed);
            }
            Err(e) => return Err(IpcError::Io(e)),
        }
        let length = u32::from_be_bytes(len_bytes) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(IpcError::MessageTooLarge(length, MAX_MESSAGE_SIZE));
        }
        let mut payload = vec![0u8; length];
        self.reader.read_exact(&mut payload).await?;
        decode_media_command(&payload)
    }
}

pub struct MediaCommandWriter<W> {
    writer: W,
}

impl<W: AsyncWrite + Unpin> MediaCommandWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub async fn write_command(&mut self, cmd: &MediaWorkerCommand) -> Result<()> {
        let buffer = encode_media_command(cmd)?;
        self.writer.write_all(&buffer).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

pub struct MediaEventReader<R> {
    reader: R,
}

impl<R: AsyncRead + Unpin> MediaEventReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub async fn read_event(&mut self) -> Result<MediaWorkerEvent> {
        let mut len_bytes = [0u8; 4];
        match self.reader.read_exact(&mut len_bytes).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(IpcError::ConnectionClosed);
            }
            Err(e) => return Err(IpcError::Io(e)),
        }
        let length = u32::from_be_bytes(len_bytes) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(IpcError::MessageTooLarge(length, MAX_MESSAGE_SIZE));
        }
        let mut payload = vec![0u8; length];
        self.reader.read_exact(&mut payload).await?;
        decode_media_event(&payload)
    }
}

pub struct MediaEventWriter<W> {
    writer: W,
}

impl<W: AsyncWrite + Unpin> MediaEventWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub async fn write_event(&mut self, event: &MediaWorkerEvent) -> Result<()> {
        let buffer = encode_media_event(event)?;
        self.writer.write_all(&buffer).await?;
        self.writer.flush().await?;
        Ok(())
    }
}
