use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use data::config::{DataSource, SourceType};
use data::queries::summary::CostRepository;

use crate::error::ServiceError;
use crate::register::Registered;

/// Registration outcome for one configured source.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    /// Registration is in progress (startup, save or reload).
    Pending,
    /// The `normalized_cost` view is registered and queryable via `source_id`.
    Registered { detected_format: String, file_count: usize },
    /// Registration failed; `reason` is human-readable.
    Skipped { reason: String },
}

/// One configured source's identity plus its registration outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceDiagnostic {
    pub id: String,
    pub name: String,
    pub configured_type: SourceType,
    pub status: SourceStatus,
}

#[derive(Default)]
struct Inner {
    /// In configured order; `default_source_id` is the first entry.
    diagnostics: Vec<SourceDiagnostic>,
    repos: HashMap<String, Arc<dyn CostRepository>>,
}

/// Runtime-mutable set of sources. Registration I/O happens *outside* the
/// lock (see `CostlyticsService::register_one`); only the bookkeeping here
/// is locked, so queries on other sources never wait on a slow S3 source.
#[derive(Default)]
pub struct SourceRegistry {
    inner: RwLock<Inner>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds `source` as `Pending` (appended if new, in place if known) and
    /// drops any repository it had until the next [`Self::set_result`].
    pub fn mark_pending(&self, source: &DataSource) {
        let mut inner = self.write();
        inner.repos.remove(&source.id);
        let diag = SourceDiagnostic {
            id: source.id.clone(),
            name: source.name.clone(),
            configured_type: source.source_type.clone(),
            status: SourceStatus::Pending,
        };
        match inner.diagnostics.iter_mut().find(|d| d.id == source.id) {
            Some(existing) => *existing = diag,
            None => inner.diagnostics.push(diag),
        }
    }

    /// Records a registration outcome. Ignored if `id` was removed meanwhile.
    pub fn set_result(&self, id: &str, result: Result<Registered, String>) {
        let mut inner = self.write();
        let Some(pos) = inner.diagnostics.iter().position(|d| d.id == id) else {
            return;
        };
        let status = match result {
            Ok(r) => {
                inner.repos.insert(id.to_string(), r.repo);
                SourceStatus::Registered {
                    detected_format: r.detected_format,
                    file_count: r.file_count,
                }
            }
            Err(reason) => {
                inner.repos.remove(id);
                SourceStatus::Skipped { reason }
            }
        };
        inner.diagnostics[pos].status = status;
    }

    pub fn remove(&self, id: &str) {
        let mut inner = self.write();
        inner.diagnostics.retain(|d| d.id != id);
        inner.repos.remove(id);
    }

    pub fn diagnostics(&self) -> Vec<SourceDiagnostic> {
        self.read().diagnostics.clone()
    }

    /// The first *configured* source (which may be skipped or pending) —
    /// the fallback for requests without `source_id`.
    pub fn default_source_id(&self) -> Option<String> {
        self.read().diagnostics.first().map(|d| d.id.clone())
    }

    /// Resolves `source_id` (or the default source) to its repository.
    pub fn repo(&self, source_id: Option<&str>) -> Result<Arc<dyn CostRepository>, ServiceError> {
        let id = match source_id {
            Some(id) => id.to_string(),
            None => self
                .default_source_id()
                .ok_or_else(|| ServiceError::bad_request("no sources configured"))?,
        };
        self.read()
            .repos
            .get(&id)
            .cloned()
            .ok_or_else(|| ServiceError::bad_request(format!("unknown source_id '{}'", id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    fn source(id: &str) -> DataSource {
        DataSource { id: id.into(), name: id.to_uppercase(), s3_uri: "x".into(), ..Default::default() }
    }

    fn registered() -> Registered {
        let pool = data::duckdb_pool::build_pool().unwrap();
        Registered {
            repo: Arc::new(data::queries::summary::DuckDbCostRepository::new(pool)),
            detected_format: "focus12".into(),
            file_count: 2,
            billing_periods: vec!["2026-08".into()],
        }
    }

    #[test]
    fn pending_then_registered_then_skipped() {
        let reg = SourceRegistry::new();
        reg.mark_pending(&source("a"));
        assert_eq!(reg.diagnostics()[0].status, SourceStatus::Pending);
        assert!(reg.repo(Some("a")).is_err());

        reg.set_result("a", Ok(registered()));
        assert_eq!(
            reg.diagnostics()[0].status,
            SourceStatus::Registered { detected_format: "focus12".into(), file_count: 2 }
        );
        assert!(reg.repo(Some("a")).is_ok());
        assert!(reg.repo(None).is_ok(), "None falls back to the first configured source");

        reg.set_result("a", Err("boom".into()));
        assert_eq!(reg.diagnostics()[0].status, SourceStatus::Skipped { reason: "boom".into() });
        assert!(reg.repo(Some("a")).is_err());
    }

    #[test]
    fn order_is_preserved_and_remove_drops_everything() {
        let reg = SourceRegistry::new();
        reg.mark_pending(&source("a"));
        reg.mark_pending(&source("b"));
        reg.mark_pending(&source("a")); // re-mark keeps position
        let ids: Vec<_> = reg.diagnostics().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["a", "b"]);

        reg.remove("a");
        assert_eq!(reg.default_source_id().as_deref(), Some("b"));
        reg.set_result("a", Ok(registered())); // late result for a removed source
        assert_eq!(reg.diagnostics().len(), 1);
    }

    #[test]
    fn repo_errors() {
        let reg = SourceRegistry::new();
        let err = reg.repo(None).err().unwrap();
        assert_eq!((err.kind, err.message.as_str()), (ErrorKind::BadRequest, "no sources configured"));
        let err = reg.repo(Some("zzz")).err().unwrap();
        assert_eq!(err.message, "unknown source_id 'zzz'");
    }
}
