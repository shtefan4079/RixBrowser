
#![deny(unsafe_code)]

pub mod browser_profile;
pub mod config;
pub mod intern;
pub mod error;
pub mod logging;
pub mod types;
pub mod url;

pub use browser_profile::BrowserProfile;
pub use config::RixConfig;
pub use intern::{intern, intern_str, intern_string, InternedStr, InternedString};
pub use error::{Result, RixError};
pub use logging::init_logging;
pub use types::{NavigationState, Point, ProcessId, Rect, Size, TabId, WindowId};
pub use url::{RixScheme, RixUrl};
