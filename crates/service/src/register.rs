use std::sync::Arc;

use data::queries::summary::CostRepository;

/// A successfully registered source: a queryable repository plus what
/// discovery/detection found.
pub struct Registered {
    pub repo: Arc<dyn CostRepository>,
    pub detected_format: String,
    pub file_count: usize,
    pub billing_periods: Vec<String>,
}

impl std::fmt::Debug for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registered")
            .field("detected_format", &self.detected_format)
            .field("file_count", &self.file_count)
            .field("billing_periods", &self.billing_periods)
            .finish_non_exhaustive()
    }
}
