use serde::Serialize;

use crate::registry::{SourceDiagnostic, SourceRegistry, SourceStatus};

/// JSON mirror of `crate::registry::SourceStatus`, `#[serde(tag = "state")]`
/// so each entry's JSON shape is either
/// `{"state": "pending"}`,
/// `{"state": "registered", "detected_format": "...", "file_count": N}` or
/// `{"state": "skipped", "reason": "..."}`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SourceStatusResponse {
    Pending,
    Registered { detected_format: String, file_count: usize },
    Skipped { reason: String },
}

impl From<&SourceStatus> for SourceStatusResponse {
    fn from(status: &SourceStatus) -> Self {
        match status {
            SourceStatus::Pending => SourceStatusResponse::Pending,
            SourceStatus::Registered { detected_format, file_count } => SourceStatusResponse::Registered {
                detected_format: detected_format.clone(),
                file_count: *file_count,
            },
            SourceStatus::Skipped { reason } => SourceStatusResponse::Skipped { reason: reason.clone() },
        }
    }
}

/// JSON mirror of `crate::registry::SourceDiagnostic`.
#[derive(Debug, Clone, Serialize)]
pub struct SourceEntry {
    pub id: String,
    pub name: String,
    pub configured_type: data::config::SourceType,
    #[serde(flatten)]
    pub status: SourceStatusResponse,
}

impl From<&SourceDiagnostic> for SourceEntry {
    fn from(d: &SourceDiagnostic) -> Self {
        SourceEntry {
            id: d.id.clone(),
            name: d.name.clone(),
            configured_type: d.configured_type.clone(),
            status: SourceStatusResponse::from(&d.status),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourcesResponse {
    pub sources: Vec<SourceEntry>,
    /// The `source_id` that `SourceRegistry::repo(None)` would pick — i.e.
    /// `SourceRegistry::default_source_id`, mirroring `repo`'s exact
    /// fallback rule (the first *configured* source, not the first
    /// *registered* one — if that source happens to be skipped,
    /// `repo(None)` itself would fail with "unknown source_id", which this
    /// field intentionally surfaces rather than papers over).
    pub default_source_id: Option<String>,
}

pub fn sources_response(registry: &SourceRegistry) -> SourcesResponse {
    SourcesResponse {
        sources: registry.diagnostics().iter().map(SourceEntry::from).collect(),
        default_source_id: registry.default_source_id(),
    }
}

pub fn entry_for(registry: &SourceRegistry, id: &str) -> Option<SourceEntry> {
    registry.diagnostics().iter().find(|d| d.id == id).map(SourceEntry::from)
}
