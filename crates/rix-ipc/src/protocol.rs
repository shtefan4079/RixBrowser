
#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::io;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Error, Debug)]
pub enum IpcError {
    #[error("I/O Error: {0}")]
    Io(#[from] io::Error),

    #[error("Postcard Serialization Error: {0}")]
    Serialization(#[from] postcard::Error),

    #[error("Connection Closed")]
    ConnectionClosed,

    #[error("Message too large: {0} bytes (max: {1})")]
    MessageTooLarge(usize, usize),

    #[error("Tab worker spawn failed: {0}")]
    SpawnFailed(String),

    #[error("IPC connection timed out")]
    AcceptTimeout,

    #[error("Tab process exited: {0}")]
    ChildExited(String),

    #[error("No such tab: {0}")]
    NoSuchTab(u32),
}

pub type Result<T> = std::result::Result<T, IpcError>;

pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IpcInputEvent {
    MouseMove { x: f32, y: f32 },
    MouseDown { x: f32, y: f32, button: u8 },
    MouseUp { x: f32, y: f32, button: u8 },
    KeyDown { key: String, code: String },
    KeyUp { key: String, code: String },
    Scroll { dx: f32, dy: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IpcMessage {
    NavigateTo {
        tab_id: u32,
        url: String,
    },
    PageLoaded {
        tab_id: u32,
        url: String,
        title: String,
    },
    InputEvent {
        tab_id: u32,
        event: IpcInputEvent,
    },
    Resize {
        tab_id: u32,
        width: u32,
        height: u32,
        scale_factor: f64,
    },
    RenderFrameReady {
        tab_id: u32,
        buffer_index: u8,
        frame_number: u64,
        width: u32,
        height: u32,
        stride: u32,
    },
    TabCreated {
        tab_id: u32,
        initial_url: String,
    },
    TabClosed {
        tab_id: u32,
    },
    CrashNotification {
        tab_id: u32,
        reason: String,
    },
    HeartbeatPing {
        sequence: u64,
    },
    HeartbeatPong {
        sequence: u64,
    },
}

pub fn encode_message(msg: &IpcMessage) -> Result<Vec<u8>> {
    let payload = postcard::to_allocvec(msg)?;
    let length = payload.len();
    if length > MAX_MESSAGE_SIZE {
        return Err(IpcError::MessageTooLarge(length, MAX_MESSAGE_SIZE));
    }
    let mut buffer = Vec::with_capacity(4 + length);
    buffer.extend_from_slice(&(length as u32).to_be_bytes());
    buffer.extend_from_slice(&payload);
    Ok(buffer)
}

pub fn decode_message(payload: &[u8]) -> Result<IpcMessage> {
    let msg = postcard::from_bytes(payload)?;
    Ok(msg)
}

pub struct IpcReader<R> {
    reader: R,
}

impl<R: AsyncRead + Unpin> IpcReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub async fn read_message(&mut self) -> Result<IpcMessage> {
        let mut len_bytes = [0u8; 4];
        match self.reader.read_exact(&mut len_bytes).await {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
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
        decode_message(&payload)
    }
}

pub struct IpcWriter<W> {
    writer: W,
}

impl<W: AsyncWrite + Unpin> IpcWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub async fn write_message(&mut self, msg: &IpcMessage) -> Result<()> {
        let encoded = encode_message(msg)?;
        self.writer.write_all(&encoded).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

pub struct IpcChannel<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> IpcChannel<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn split(self) -> (IpcReader<tokio::io::ReadHalf<S>>, IpcWriter<tokio::io::WriteHalf<S>>) {
        let (r, w) = tokio::io::split(self.stream);
        (IpcReader::new(r), IpcWriter::new(w))
    }
}

#[cfg(unix)]
pub fn bind_unix_listener(
    socket_path: &std::path::Path,
) -> Result<tokio::net::UnixListener> {
    let _ = std::fs::remove_file(socket_path);
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    tokio::net::UnixListener::bind(socket_path).map_err(IpcError::Io)
}
