
use std::sync::Once;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static INIT_LOGGER_ONCE: Once = Once::new();

pub fn init_logging(default_level: &str) {
    INIT_LOGGER_ONCE.call_once(|| {
        let filter = EnvFilter::try_from_env("RIX_LOG")
            .or_else(|_| EnvFilter::try_from_env("RUST_LOG"))
            .unwrap_or_else(|_| EnvFilter::new(default_level));

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_target(true).with_thread_ids(true))
            .try_init()
            .ok();
    });
}

pub fn init_logging_to_stderr(default_level: &str) {
    INIT_LOGGER_ONCE.call_once(|| {
        let filter = EnvFilter::try_from_env("RIX_LOG")
            .or_else(|_| EnvFilter::try_from_env("RUST_LOG"))
            .unwrap_or_else(|_| EnvFilter::new(default_level));

        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_writer(std::io::stderr),
            )
            .try_init()
            .ok();
    });
}
