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
    /// Bumped by every [`SourceRegistry::mark_pending`] on this id. Lets a
    /// caller that raced a later `mark_pending`/`remove` for the same id
    /// (e.g. a background `register_all` racing a Settings save/delete)
    /// detect that its own in-flight registration is stale and skip
    /// applying its result — see [`SourceRegistry::set_result_if_current`].
    generation: u64,
}

#[derive(Default)]
struct Inner {
    /// In configured order; `default_source_id` is the first entry.
    diagnostics: Vec<SourceDiagnostic>,
    repos: HashMap<String, Arc<dyn CostRepository>>,
    /// Registry-wide monotonically increasing counter; every
    /// [`SourceRegistry::mark_pending`] call draws its generation from here
    /// and increments it. Because generations are drawn from one shared
    /// counter (not per-id), they never repeat across ids or across a
    /// `remove` + re-`mark_pending` of the same id — unlike a per-id counter,
    /// which resets to 0 for an id it no longer knows about (see
    /// `mark_pending`'s old behavior), letting a stale in-flight result from
    /// before the removal collide with a fresh generation after re-adding.
    next_generation: u64,
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

    /// Adds `source` as `Pending` (appended if new, in place if known),
    /// drops any repository it had until the next [`Self::set_result`], and
    /// returns this id's new generation — pass it to
    /// [`Self::set_result_if_current`] so a result computed from a since-
    /// superseded `mark_pending` (a re-save, a reload, or another
    /// `mark_pending` for the same id) is ignored rather than clobbering a
    /// newer one.
    pub fn mark_pending(&self, source: &DataSource) -> u64 {
        let mut inner = self.write();
        inner.repos.remove(&source.id);
        let generation = inner.next_generation;
        inner.next_generation += 1;
        let diag = SourceDiagnostic {
            id: source.id.clone(),
            name: source.name.clone(),
            configured_type: source.source_type.clone(),
            status: SourceStatus::Pending,
            generation,
        };
        match inner.diagnostics.iter_mut().find(|d| d.id == source.id) {
            Some(existing) => *existing = diag,
            None => inner.diagnostics.push(diag),
        }
        generation
    }

    /// Applies `result` to `id`'s diagnostic at `pos`, shared by
    /// [`Self::set_result`] and [`Self::set_result_if_current`].
    fn apply_result(inner: &mut Inner, pos: usize, id: &str, result: Result<Registered, String>) {
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

    /// Records a registration outcome unconditionally. Ignored if `id` was
    /// removed meanwhile. Used by tests and by call sites that don't race a
    /// concurrent `mark_pending`/`remove` for the same id; prefer
    /// [`Self::set_result_if_current`] wherever they might.
    pub fn set_result(&self, id: &str, result: Result<Registered, String>) {
        let mut inner = self.write();
        let Some(pos) = inner.diagnostics.iter().position(|d| d.id == id) else {
            return;
        };
        Self::apply_result(&mut inner, pos, id, result);
    }

    /// Records a registration outcome only if `id` still exists and its
    /// generation is still `generation` (i.e. no later `mark_pending` or a
    /// `remove` has superseded the `mark_pending` call that produced
    /// `generation`). Otherwise the result is silently discarded as stale.
    pub fn set_result_if_current(&self, id: &str, generation: u64, result: Result<Registered, String>) {
        let mut inner = self.write();
        let Some(pos) = inner.diagnostics.iter().position(|d| d.id == id && d.generation == generation) else {
            return;
        };
        Self::apply_result(&mut inner, pos, id, result);
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
    fn set_result_if_current_ignores_a_stale_generation() {
        let reg = SourceRegistry::new();
        let gen0 = reg.mark_pending(&source("a"));
        let gen1 = reg.mark_pending(&source("a")); // supersedes gen0 (e.g. a reload/re-save)
        assert_ne!(gen0, gen1);

        // The stale (gen0) result is discarded: still Pending, no repo.
        reg.set_result_if_current("a", gen0, Ok(registered()));
        assert_eq!(reg.diagnostics()[0].status, SourceStatus::Pending);
        assert!(reg.repo(Some("a")).is_err());

        // The current (gen1) result is applied.
        reg.set_result_if_current("a", gen1, Ok(registered()));
        assert!(matches!(reg.diagnostics()[0].status, SourceStatus::Registered { .. }));
        assert!(reg.repo(Some("a")).is_ok());
    }

    #[test]
    fn set_result_if_current_does_not_re_add_a_removed_source() {
        let reg = SourceRegistry::new();
        let gen0 = reg.mark_pending(&source("a"));
        reg.remove("a");
        reg.set_result_if_current("a", gen0, Ok(registered()));
        assert!(reg.diagnostics().is_empty());
        assert!(reg.repo(Some("a")).is_err());
    }

    /// A generation counter that were per-id (rather than registry-wide)
    /// would reset to 0 after `remove`, letting a stale in-flight result
    /// from before the removal collide with the fresh generation issued
    /// after re-adding the same id. The registry-wide counter (this test)
    /// guarantees generations never repeat, even across a remove/re-add.
    #[test]
    fn generation_after_remove_and_re_add_never_collides_with_a_stale_one() {
        let reg = SourceRegistry::new();
        let gen1 = reg.mark_pending(&source("a"));
        reg.remove("a");
        let gen2 = reg.mark_pending(&source("a"));
        assert_ne!(gen1, gen2);

        // The stale gen1 result (from before the remove) must be ignored,
        // leaving the re-added "a" still Pending.
        reg.set_result_if_current("a", gen1, Ok(registered()));
        assert_eq!(reg.diagnostics()[0].status, SourceStatus::Pending);
        assert!(reg.repo(Some("a")).is_err());

        // The current (gen2) result still applies normally.
        reg.set_result_if_current("a", gen2, Ok(registered()));
        assert!(matches!(reg.diagnostics()[0].status, SourceStatus::Registered { .. }));
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
