
#![deny(unsafe_code)]

pub mod aac;
pub mod h264;
pub mod opus;
pub mod pcm;
pub mod registry;
pub mod traits;
pub mod vorbis;
pub mod vp8;

pub use opus::OpusAudioDecoder;
pub use pcm::{PcmAudioDecoder, PcmFormat};
pub use registry::CodecRegistry;
pub use traits::{AudioDecoder, VideoDecoder};
pub use vorbis::VorbisAudioDecoder;
pub use vp8::Vp8VideoDecoder;
