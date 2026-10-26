
#![deny(unsafe_code)]

pub mod audio_sink;
pub mod clock;
pub mod decoder;
pub mod demuxer;
pub mod engine;
pub mod error;
pub mod sample;
pub mod video_sink;

pub use audio_sink::AudioRenderer;
pub use clock::{FrameAction, MediaClock, LATE_FRAME_DROP_THRESHOLD_SECONDS};
pub use decoder::{
    AudioDecoder, CodecRegistry, OpusAudioDecoder, PcmAudioDecoder, PcmFormat, VideoDecoder,
    Vp8VideoDecoder,
};
pub use demuxer::{create_demuxer_for_mime, Fmp4Demuxer, MediaDemuxer, WebmDemuxer};
pub use engine::{EngineTickOutput, MediaSession, PlaybackEngine, MAX_SESSION_BYTES, MAX_SOURCE_BUFFER_BYTES};
pub use error::{MediaError, MediaResult};
pub use sample::{
    AudioConfig, DecodedAudioFrame, DecodedVideoFrame, MediaSample, MediaTrack, TrackType,
    VideoConfig,
};
pub use video_sink::{VideoFrameStatus, VideoRenderer};
