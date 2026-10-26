
#![deny(unsafe_code)]

pub mod fmp4;
pub mod traits;
pub mod webm;

pub use fmp4::Fmp4Demuxer;
pub use traits::{merge_time_ranges, MediaDemuxer};
pub use webm::WebmDemuxer;

pub fn create_demuxer_for_mime(mime_type: &str) -> Option<Box<dyn MediaDemuxer>> {
    let lower = mime_type.to_ascii_lowercase();
    let container = lower.split(';').next().unwrap_or("").trim();
    match container {
        "video/mp4" | "audio/mp4" | "application/mp4" => Some(Box::new(Fmp4Demuxer::new())),
        "video/webm" | "audio/webm" => Some(Box::new(WebmDemuxer::new())),
        _ => None,
    }
}
