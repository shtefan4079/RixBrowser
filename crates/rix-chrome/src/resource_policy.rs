use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct NavigationResourcePolicy {
    pub max_document_bytes: usize,
    pub max_stylesheet_bytes: usize,
    pub max_script_bytes: usize,
    pub max_source_bytes: usize,
    pub max_resources: usize,
    pub concurrent_resources: usize,
    pub resource_timeout: Duration,
}

impl Default for NavigationResourcePolicy {
    fn default() -> Self {
        Self {
            max_document_bytes: 16 * 1024 * 1024,
            max_stylesheet_bytes: 8 * 1024 * 1024,
            max_script_bytes: 16 * 1024 * 1024,
            max_source_bytes: 32 * 1024 * 1024,
            max_resources: 256,
            concurrent_resources: 32,
            resource_timeout: Duration::from_secs(30),
        }
    }
}

pub(crate) struct SourceBudget {
    maximum: usize,
    retained: usize,
}
impl SourceBudget {
    pub(crate) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            retained: 0,
        }
    }
    pub(crate) fn retain(&mut self, bytes: usize) -> bool {
        if bytes > self.maximum.saturating_sub(self.retained) {
            return false;
        }
        self.retained += bytes;
        true
    }
}
