
use serde::{Deserialize, Serialize};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NavigationTiming {
    pub time_origin_ms: f64,
    pub fetch_start_ms: f64,
    pub request_start_ms: f64,
    pub response_start_ms: f64,
    pub response_end_ms: f64,
    pub redirect_start_ms: Option<f64>,
    pub redirect_end_ms: Option<f64>,
    pub redirect_count: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct FetchClock {
    started: Instant,
    time_origin_ms: f64,
}

pub(crate) struct CompletedTiming {
    pub(crate) navigation: NavigationTiming,
    pub(crate) started: Instant,
}

impl FetchClock {
    pub(crate) fn new() -> Self {
        let started = Instant::now();
        let time_origin_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;
        Self {
            started,
            time_origin_ms,
        }
    }

    pub(crate) fn elapsed_ms(self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub(crate) fn finish(
        self,
        fetch_start_ms: f64,
        request_start_ms: f64,
        response_start_ms: f64,
    ) -> CompletedTiming {
        CompletedTiming {
            started: self.started,
            navigation: NavigationTiming {
                time_origin_ms: self.time_origin_ms,
                fetch_start_ms,
                request_start_ms,
                response_start_ms,
                response_end_ms: self.elapsed_ms(),
                redirect_start_ms: None,
                redirect_end_ms: None,
                redirect_count: 0,
            },
        }
    }
}
