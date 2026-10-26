
#![deny(unsafe_code)]

use crate::error::MediaResult;
use crate::sample::{MediaSample, MediaTrack};

pub trait MediaDemuxer: Send {
    fn append_data(&mut self, chunk: &[u8]) -> MediaResult<()>;

    fn is_init_complete(&self) -> bool;

    fn tracks(&self) -> &[MediaTrack];

    fn duration(&self) -> Option<f64>;

    fn poll_sample(&mut self) -> Option<MediaSample>;

    fn buffered_ranges(&self) -> Vec<(f64, f64)>;

    fn seek(&mut self, target_time: f64) -> MediaResult<()>;

    fn remove_range(&mut self, start: f64, end: f64) {
        let _ = (start, end);
    }

    fn reset(&mut self);
}

pub fn merge_time_ranges(ranges: &mut Vec<(f64, f64)>) {
    if ranges.is_empty() {
        return;
    }
    ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];

    for &r in &ranges[1..] {
        if r.0 <= current.1 + 0.05 {
            current.1 = current.1.max(r.1);
        } else {
            merged.push(current);
            current = r;
        }
    }
    merged.push(current);
    *ranges = merged;
}
