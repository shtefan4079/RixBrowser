
#![deny(unsafe_code)]

use crate::error::MediaResult;
use crate::sample::{DecodedAudioFrame, DecodedVideoFrame, MediaSample};

pub trait AudioDecoder: Send {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Vec<DecodedAudioFrame>>;

    fn flush(&mut self) -> MediaResult<Vec<DecodedAudioFrame>>;

    fn reset(&mut self);
}

pub trait VideoDecoder: Send {
    fn decode_packet(&mut self, sample: &MediaSample) -> MediaResult<Option<DecodedVideoFrame>>;

    fn flush(&mut self) -> MediaResult<Option<DecodedVideoFrame>>;

    fn reset(&mut self);
}
