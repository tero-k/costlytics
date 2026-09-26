use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use data::config::{AppConfig, CostGuardConfig, DataSource, S3AuthConfig};
use serde::{Deserialize, Serialize};

use crate::cost::{self, EstimateRequest, EstimateResponse};
use crate::error::ServiceError;
use crate::register::register_source;
use crate::registry::SourceRegistry;
use crate::secrets::SecretStore;
use crate::sources::{entry_for, sources_response, SourceEntry, SourcesResponse};

#[derive(Debug, Clone, Serialize)]
pub struct SourceSettings {
    #[serde(flatten)]
    pub source: DataSource,
    /// Whether an access-key secret is stored in the keychain (never the secret itself).
    pub has_secret: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsResponse {
    pub sources: Vec<SourceSettings>,
    pub cost_guard: CostGuardConfig,
}

#[derive(Deserialize)]
pub struct SaveSourceRequest {
    pub source: DataSource,
    /// New access-key secret; `None`/empty keeps the stored one.
    #[serde(default)]
    pub secret: Option<String>,
    pub is_new: bool,
}

/// Manual impl (rather than `#[derive(Debug)]`) so a stray `{:?}` log/panic
/// message never prints the raw access-key secret.
impl std::fmt::Debug for SaveSourceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveSourceRequest")
            .field("source", &self.source)
            .field("secret", &self.secret.as_ref().map(|_| "***"))
            .field("is_new", &self.is_new)
            .finish()
    }
}

#[derive(Deserialize)]
pub struct TestSourceRequest {
    pub source: DataSource,
    #[serde(default)]
    pub secret: Option<String>,
}

/// See `SaveSourceRequest`'s manual `Debug` impl for why.
impl std::fmt::Debug for TestSourceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestSourceRequest")
            .field("source", &self.source)
            .field("secret", &self.secret.as_ref().map(|_| "***"))
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct SourceIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestSourceResponse {
    pub detected_format: String,
    pub file_count: usize,
    pub billing_periods: Vec<String>,
}

/// The whole backend: configured sources (persisted to `settings_path`),
/// their runtime registrations, and the secret store.
pub struct CostlyticsService {
    pub registry: SourceRegistry,
    config: RwLock<AppConfig>,
    settings_path: Option<PathBuf>,
    secrets: Arc<dyn SecretStore>,
    /// Serializes `save_source`/`delete_source`'s read-modify-write of
    /// `config` (and the keychain write that goes with it), so two
    /// concurrent calls can't both read the same base and silently drop
    /// each other's change. Held only across the read-check-persist-write
    /// section, never across registration I/O (see below).
    mutation: Mutex<()>,
    /// Dev/test harness only: estimate local sources as if they were S3, so
    /// the cost-warning flow can be exercised against local fixtures.
    treat_local_as_remote: bool,
}

fn validate_source(s: &DataSource) -> Result<(), ServiceError> {
    let id_ok = !s.id.is_empty()
        && s.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !id_ok {
        return Err(ServiceError::bad_request("id must be non-empty and use only a-z, 0-9, '-' or '_'"));
    }
    if s.name.trim().is_empty() {
        return Err(ServiceError::bad_request("name is required"));
    }
    let location = s.s3_uri.trim();
    if location.is_empty() {
        return Err(ServiceError::bad_request("location is required"));
    }
    if let Some(rest) = location.strip_prefix("s3://") {
        if rest.trim_matches('/').is_empty() {
            return Err(ServiceError::bad_request("S3 URI must include a bucket, e.g. s3://my-bucket/exports/data"));
        }
    }
    if let S3AuthConfig::AccessKey { key_id } = &s.auth {
        if key_id.trim().is_empty() {
            return Err(ServiceError::bad_request("access key id is required"));
        }
    }
    Ok(())
}

impl CostlyticsService {
    pub fn new(config: AppConfig, settings_path: Option<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        let registry = SourceRegistry::new();
        for source in &config.sources {
            registry.mark_pending(source);
        }
        Self {
            registry,
            config: RwLock::new(config),
            settings_path,
            secrets,
            mutation: Mutex::new(()),
            treat_local_as_remote: false,
        }
    }

    /// See the `treat_local_as_remote` field. Never set by the desktop app.
    pub fn with_local_treated_as_remote(mut self, on: bool) -> Self {
        self.treat_local_as_remote = on;
        self
    }

    fn config(&self) -> AppConfig {
        self.config.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn find(&self, id: &str) -> Option<DataSource> {
        self.config().sources.into_iter().find(|s| s.id == id)
    }

    fn stored_secret(&self, id: &str) -> Option<String> {
        self.secrets.get(id).unwrap_or_else(|e| {
            tracing::warn!(source_id = %id, error = %e, "could not read secret from keychain");
            None
        })
    }

    /// The secret to register `source` with: the newly supplied one, else
    /// the stored one. Required for access-key auth.
    fn resolve_secret(&self, source: &DataSource, supplied: Option<&str>) -> Result<Option<String>, ServiceError> {
        match &source.auth {
            S3AuthConfig::CredentialChain => Ok(None),
            S3AuthConfig::AccessKey { .. } => match supplied.map(str::trim).filter(|s| !s.is_empty()) {
                Some(s) => Ok(Some(s.to_string())),
                None => self
                    .stored_secret(&source.id)
                    .map(Some)
                    .ok_or_else(|| ServiceError::bad_request("secret access key is required")),
            },
        }
    }

    fn persist(&self, config: &AppConfig) -> Result<(), ServiceError> {
        if let Some(path) = &self.settings_path {
            config.save(path).map_err(|e| ServiceError::internal(format!("failed to save settings: {e}")))?;
        }
        Ok(())
    }

    /// Blocking: (re)registers one source, updating the registry, given a
    /// generation already captured by a prior `mark_pending` (typically
    /// while holding `mutation` — see `save_source`/`reload_source`). Uses
    /// that generation so a result made stale by a concurrent
    /// `mark_pending`/`remove` for the same id (another save, reload, or
    /// `register_all` racing a Settings change) is discarded instead of
    /// clobbering a newer outcome — see `SourceRegistry::set_result_if_current`.
    fn register_with_generation(&self, source: &DataSource, secret: Option<&str>, generation: u64) {
        let result = register_source(source, secret);
        match &result {
            Ok(r) => tracing::info!(source_id = %source.id, file_count = r.file_count, format = %r.detected_format, "source registered"),
            Err(reason) => tracing::warn!(source_id = %source.id, %reason, "source skipped"),
        }
        self.registry.set_result_if_current(&source.id, generation, result);
    }

    fn entry(&self, id: &str) -> Result<SourceEntry, ServiceError> {
        entry_for(&self.registry, id).ok_or_else(|| ServiceError::not_found(format!("unknown source '{id}'")))
    }

    /// Blocking: registers every configured source, in order. Runs in the
    /// background at startup, so it may race a Settings save/delete for the
    /// same id. For each source, re-checks (under `mutation`) that the
    /// source is still configured unchanged before calling `mark_pending`,
    /// then registers outside the lock and applies the result only if it's
    /// still current (`SourceRegistry::set_result_if_current`) — so a
    /// source deleted mid-registration can't be resurrected by a stale
    /// result, and a save/reload that raced this one always wins.
    pub fn register_all(&self) {
        for source in self.config().sources {
            let generation = {
                let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
                let current = self.config();
                match current.sources.iter().find(|s| s.id == source.id) {
                    Some(s) if *s == source => Some(self.registry.mark_pending(&source)),
                    _ => None,
                }
            };
            let Some(generation) = generation else {
                tracing::info!(source_id = %source.id, "source changed or removed before registration; skipping");
                continue;
            };

            let secret = match source.auth {
                S3AuthConfig::AccessKey { .. } => self.stored_secret(&source.id),
                S3AuthConfig::CredentialChain => None,
            };
            self.register_with_generation(&source, secret.as_deref(), generation);
        }
    }

    pub fn sources(&self) -> SourcesResponse {
        sources_response(&self.registry)
    }

    pub fn settings(&self) -> SettingsResponse {
        let sources = self
            .config()
            .sources
            .into_iter()
            .map(|source| {
                let has_secret = matches!(source.auth, S3AuthConfig::AccessKey { .. })
                    && self.stored_secret(&source.id).is_some();
                SourceSettings { source, has_secret }
            })
            .collect();
        SettingsResponse { sources, cost_guard: self.config().cost_guard }
    }

    pub fn cost_estimate(&self, req: EstimateRequest) -> Result<EstimateResponse, ServiceError> {
        let guard = self.config().cost_guard;
        cost::cost_estimate(&self.registry, &guard, self.treat_local_as_remote, req)
    }

    pub fn save_cost_guard(&self, guard: CostGuardConfig) -> Result<CostGuardConfig, ServiceError> {
        let values = [
            ("soft_limit_usd", guard.soft_limit_usd),
            ("hard_limit_usd", guard.hard_limit_usd),
            ("egress_usd_per_gb", guard.egress_usd_per_gb),
            ("get_usd_per_1000", guard.get_usd_per_1000),
        ];
        for (name, value) in values {
            if !value.is_finite() || value < 0.0 {
                return Err(ServiceError::bad_request(format!("{name} must be a non-negative number")));
            }
        }
        if guard.hard_limit_usd < guard.soft_limit_usd {
            return Err(ServiceError::bad_request("hard limit must be at least the soft limit"));
        }
        // See `save_source` for why the read-modify-write is serialized.
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
        let mut config = self.config();
        config.cost_guard = guard.clone();
        self.persist(&config)?;
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
        Ok(guard)
    }

    /// Best-effort restore of a source's keychain entry to `previous` after
    /// a failed `persist`, so we never leave the keychain out of sync with
    /// the config file. Never logs the secret itself.
    fn restore_secret(&self, id: &str, previous: Option<&str>) {
        let result = match previous {
            Some(s) => self.secrets.set(id, s),
            None => self.secrets.delete(id),
        };
        if let Err(e) = result {
            tracing::warn!(source_id = %id, error = %e, "failed to roll back keychain entry after a failed save");
        }
    }

    pub fn save_source(&self, req: SaveSourceRequest) -> Result<SourceEntry, ServiceError> {
        let mut source = req.source;
        source.s3_uri = source.s3_uri.trim().to_string();
        validate_source(&source)?;

        // Held from the existence check through the in-memory write-back so
        // two concurrent saves can't both read the same base config and
        // silently drop each other's change (a lost update). Released
        // before registration I/O runs, so a slow S3 source never blocks
        // other saves/deletes.
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());

        let exists = self.find(&source.id).is_some();
        if req.is_new && exists {
            return Err(ServiceError::conflict(format!("a source with id '{}' already exists", source.id)));
        }
        if !req.is_new && !exists {
            return Err(ServiceError::not_found(format!("unknown source '{}'", source.id)));
        }
        let secret = self.resolve_secret(&source, req.secret.as_deref())?;

        let previous = self.stored_secret(&source.id);
        match &secret {
            Some(s) => self.secrets.set(&source.id, s),
            None => self.secrets.delete(&source.id),
        }
        .map_err(|e| ServiceError::internal(format!("failed to update keychain: {e}")))?;

        let mut config = self.config();
        match config.sources.iter_mut().find(|s| s.id == source.id) {
            Some(existing) => *existing = source.clone(),
            None => config.sources.push(source.clone()),
        }
        if let Err(e) = self.persist(&config) {
            self.restore_secret(&source.id, previous.as_deref());
            return Err(e);
        }
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
        // Captured while still holding `mutation`, so a concurrent save/
        // reload/`register_all` pass for this id can't be sandwiched
        // between our config write-back and this `mark_pending` and have
        // its own (older) generation look newer than a mark_pending that
        // logically happens after it. Slow registration I/O itself still
        // runs after the lock is released, below.
        let generation = self.registry.mark_pending(&source);
        drop(_guard);

        self.register_with_generation(&source, secret.as_deref(), generation);
        self.entry(&source.id)
    }

    pub fn delete_source(&self, req: SourceIdRequest) -> Result<SourcesResponse, ServiceError> {
        // See `save_source` for why this is held only across the
        // read-check-persist-write section.
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());

        let mut config = self.config();
        let before = config.sources.len();
        config.sources.retain(|s| s.id != req.id);
        if config.sources.len() == before {
            return Err(ServiceError::not_found(format!("unknown source '{}'", req.id)));
        }
        self.persist(&config)?;
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
        if let Err(e) = self.secrets.delete(&req.id) {
            tracing::warn!(source_id = %req.id, error = %e, "could not delete secret from keychain");
        }
        self.registry.remove(&req.id);
        drop(_guard);
        Ok(self.sources())
    }

    pub fn test_source(&self, req: TestSourceRequest) -> Result<TestSourceResponse, ServiceError> {
        let mut source = req.source;
        source.s3_uri = source.s3_uri.trim().to_string();
        validate_source(&source)?;
        let secret = self.resolve_secret(&source, req.secret.as_deref())?;
        let r = register_source(&source, secret.as_deref()).map_err(ServiceError::bad_request)?;
        Ok(TestSourceResponse {
            detected_format: r.detected_format,
            file_count: r.file_count,
            billing_periods: r.billing_periods,
        })
    }

    pub fn reload_source(&self, req: SourceIdRequest) -> Result<SourceEntry, ServiceError> {
        // Held only across the existence check and `mark_pending`, mirroring
        // `save_source` — see its comment on `mutation` — so a concurrent
        // save/delete/`register_all` pass for this id can't race the
        // generation this reload captures. Released before registration I/O.
        let (source, generation) = {
            let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
            let source = self.find(&req.id).ok_or_else(|| ServiceError::not_found(format!("unknown source '{}'", req.id)))?;
            let generation = self.registry.mark_pending(&source);
            (source, generation)
        };
        let secret = match source.auth {
            S3AuthConfig::AccessKey { .. } => self.stored_secret(&source.id),
            S3AuthConfig::CredentialChain => None,
        };
        self.register_with_generation(&source, secret.as_deref(), generation);
        self.entry(&source.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::registry::SourceStatus;
    use crate::secrets::MemorySecretStore;
    use data::config::{AppConfig, DataSource, S3AuthConfig};

    struct Fixture {
        _dir: tempfile::TempDir,
        fixture_path: String,
        settings_path: PathBuf,
        secrets: Arc<MemorySecretStore>,
        svc: CostlyticsService,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("focus12");
        data::fixtures::generate_focus12_fixture(&data_dir).unwrap();
        let settings_path = dir.path().join("settings.toml");
        let secrets = Arc::new(MemorySecretStore::default());
        let svc = CostlyticsService::new(AppConfig::default(), Some(settings_path.clone()), secrets.clone());
        Fixture {
            fixture_path: data_dir.to_str().unwrap().to_string(),
            _dir: dir,
            settings_path,
            secrets,
            svc,
        }
    }

    fn local(id: &str, path: &str) -> DataSource {
        DataSource { id: id.into(), name: format!("Source {id}"), s3_uri: path.into(), ..Default::default() }
    }

    fn save(f: &Fixture, source: DataSource, secret: Option<&str>, is_new: bool) -> Result<SourceEntry, ServiceError> {
        f.svc.save_source(SaveSourceRequest { source, secret: secret.map(String::from), is_new })
    }

    #[test]
    fn new_marks_configured_sources_pending_until_registered() {
        let f = fixture();
        let cfg = AppConfig { sources: vec![local("a", &f.fixture_path)], ..Default::default() };
        let svc = CostlyticsService::new(cfg, None, Arc::new(MemorySecretStore::default()));
        assert_eq!(svc.registry.diagnostics()[0].status, SourceStatus::Pending);
        svc.register_all();
        assert!(matches!(svc.registry.diagnostics()[0].status, SourceStatus::Registered { .. }));
    }

    #[test]
    fn save_new_source_persists_and_registers() {
        let f = fixture();
        let entry = save(&f, local("a", &f.fixture_path), None, true).unwrap();
        assert!(matches!(entry.status, crate::sources::SourceStatusResponse::Registered { .. }), "{entry:?}");
        let on_disk = AppConfig::load(f.settings_path.to_str().unwrap()).unwrap();
        assert_eq!(on_disk.sources[0].id, "a");
        assert_eq!(f.svc.sources().default_source_id.as_deref(), Some("a"));
    }

    #[test]
    fn save_rejects_duplicates_unknown_ids_and_bad_input() {
        let f = fixture();
        save(&f, local("a", &f.fixture_path), None, true).unwrap();
        assert_eq!(save(&f, local("a", &f.fixture_path), None, true).unwrap_err().kind, ErrorKind::Conflict);
        assert_eq!(save(&f, local("zz", &f.fixture_path), None, false).unwrap_err().kind, ErrorKind::NotFound);
        assert_eq!(save(&f, local("Bad Id", &f.fixture_path), None, true).unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(save(&f, local("b", "  "), None, true).unwrap_err().kind, ErrorKind::BadRequest);
        assert_eq!(save(&f, local("c", "s3://"), None, true).unwrap_err().kind, ErrorKind::BadRequest);
    }

    #[test]
    fn access_key_secret_goes_to_secret_store_never_to_file() {
        let f = fixture();
        let mut s = local("k", &f.fixture_path); // local path: auth is stored but unused, so no network
        s.auth = S3AuthConfig::AccessKey { key_id: "AKIAEXAMPLE".into() };

        let err = save(&f, s.clone(), None, true).unwrap_err();
        assert_eq!(err.message, "secret access key is required");

        save(&f, s.clone(), Some("TOPSECRET"), true).unwrap();
        assert_eq!(f.secrets.get("k").unwrap().as_deref(), Some("TOPSECRET"));
        assert!(!std::fs::read_to_string(&f.settings_path).unwrap().contains("TOPSECRET"));
        assert!(f.svc.settings().sources[0].has_secret);

        // Editing without re-entering the secret keeps the stored one.
        s.name = "Renamed".into();
        save(&f, s.clone(), None, false).unwrap();
        assert_eq!(f.secrets.get("k").unwrap().as_deref(), Some("TOPSECRET"));

        // Switching to the credential chain deletes it.
        s.auth = S3AuthConfig::CredentialChain;
        save(&f, s, None, false).unwrap();
        assert_eq!(f.secrets.get("k").unwrap(), None);
    }

    #[test]
    fn delete_removes_config_secret_and_registry_entry() {
        let f = fixture();
        let mut s = local("d", &f.fixture_path);
        s.auth = S3AuthConfig::AccessKey { key_id: "AKIA".into() };
        save(&f, s, Some("x"), true).unwrap();
        let resp = f.svc.delete_source(SourceIdRequest { id: "d".into() }).unwrap();
        assert!(resp.sources.is_empty());
        assert_eq!(f.secrets.get("d").unwrap(), None);
        assert!(AppConfig::load(f.settings_path.to_str().unwrap()).unwrap().sources.is_empty());
        assert_eq!(f.svc.delete_source(SourceIdRequest { id: "d".into() }).unwrap_err().kind, ErrorKind::NotFound);
    }

    #[test]
    fn test_source_reports_findings_without_saving() {
        let f = fixture();
        let r = f.svc.test_source(TestSourceRequest { source: local("t", &f.fixture_path), secret: None }).unwrap();
        assert_eq!(r.detected_format, "focus12");
        assert!(r.file_count > 0);
        assert!(f.svc.sources().sources.is_empty());
        assert!(!f.settings_path.exists());
    }

    #[test]
    fn reload_re_registers() {
        let f = fixture();
        save(&f, local("r", &f.fixture_path), None, true).unwrap();
        let entry = f.svc.reload_source(SourceIdRequest { id: "r".into() }).unwrap();
        assert!(matches!(entry.status, crate::sources::SourceStatusResponse::Registered { .. }));
    }

    #[test]
    fn failed_persist_rolls_back_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("focus12");
        data::fixtures::generate_focus12_fixture(&data_dir).unwrap();
        // No such directory exists, so `AppConfig::save` fails.
        let settings_path = dir.path().join("does-not-exist").join("settings.toml");
        let secrets = Arc::new(MemorySecretStore::default());
        let svc = CostlyticsService::new(AppConfig::default(), Some(settings_path), secrets.clone());

        let mut s = local("k", data_dir.to_str().unwrap());
        s.auth = S3AuthConfig::AccessKey { key_id: "AKIA".into() };
        let err = svc.save_source(SaveSourceRequest { source: s, secret: Some("TOPSECRET".into()), is_new: true });
        assert!(err.is_err());
        assert_eq!(secrets.get("k").unwrap(), None);
    }

    #[test]
    fn concurrent_saves_do_not_lose_updates() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("focus12");
        data::fixtures::generate_focus12_fixture(&data_dir).unwrap();
        let settings_path = dir.path().join("settings.toml");
        let secrets = Arc::new(MemorySecretStore::default());
        let svc = Arc::new(CostlyticsService::new(
            AppConfig::default(),
            Some(settings_path.clone()),
            secrets,
        ));
        let fixture_path = data_dir.to_str().unwrap().to_string();

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let svc = svc.clone();
                let fixture_path = fixture_path.clone();
                std::thread::spawn(move || {
                    let id = format!("s{i}");
                    svc.save_source(SaveSourceRequest {
                        source: local(&id, &fixture_path),
                        secret: None,
                        is_new: true,
                    })
                    .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let mut ids: Vec<_> = svc.settings().sources.into_iter().map(|s| s.source.id).collect();
        ids.sort();
        assert_eq!(ids, (0..8).map(|i| format!("s{i}")).collect::<Vec<_>>());

        let on_disk = AppConfig::load(settings_path.to_str().unwrap()).unwrap();
        let mut disk_ids: Vec<_> = on_disk.sources.into_iter().map(|s| s.id).collect();
        disk_ids.sort();
        assert_eq!(disk_ids, (0..8).map(|i| format!("s{i}")).collect::<Vec<_>>());
    }

    /// (a) `register_one` discards a stale generation's result. Simulated
    /// by re-marking the source pending (bumping its generation) between
    /// two calls to `register_with_generation` for the same id, mirroring
    /// what a concurrent reload/save would do to a slower `register_all` pass.
    #[test]
    fn register_with_generation_ignores_a_result_made_stale_by_a_concurrent_mark_pending() {
        let f = fixture();
        let source = local("a", &f.fixture_path);

        // First registration.
        let first_generation = f.svc.registry.mark_pending(&source);
        f.svc.register_with_generation(&source, None, first_generation);
        assert!(matches!(
            f.svc.registry.diagnostics()[0].status,
            crate::registry::SourceStatus::Registered { .. }
        ));

        // A concurrent operation (e.g. a reload) bumps the generation via
        // `mark_pending` directly, without registering yet.
        f.svc.registry.mark_pending(&source);
        assert_eq!(f.svc.registry.diagnostics()[0].status, crate::registry::SourceStatus::Pending);

        // `register_with_generation`'s own generation (captured before this
        // point) is now stale; applying it must not clobber the newer
        // `Pending` state with an outcome computed for the superseded
        // generation.
        f.svc.registry.set_result_if_current(
            "a",
            first_generation,
            Err("stale result from the first registration".into()),
        );
        assert_eq!(
            f.svc.registry.diagnostics()[0].status,
            crate::registry::SourceStatus::Pending,
            "a stale generation's result must be discarded, not applied"
        );
    }

    /// (b) A result for a source removed after `mark_pending` doesn't
    /// re-add it to the registry.
    #[test]
    fn result_for_a_source_removed_after_mark_pending_does_not_re_add_it() {
        let f = fixture();
        let source = local("a", &f.fixture_path);
        let generation = f.svc.registry.mark_pending(&source);

        f.svc.registry.remove("a");
        assert!(f.svc.registry.diagnostics().is_empty());

        // A registration that was already in flight for "a" completes
        // after it was removed (e.g. by `delete_source`); its result must
        // not resurrect the diagnostic.
        let result = register_source(&source, None);
        f.svc.registry.set_result_if_current("a", generation, result);
        assert!(f.svc.registry.diagnostics().is_empty());
        assert!(f.svc.sources().sources.is_empty());
    }

    /// (c) If a source is deleted from the config before `register_all`
    /// reaches it, it never appears in the registry. Uses the `mutation`
    /// lock (held here before spawning the `register_all` thread) to force
    /// the deletion to land before `register_all`'s per-source re-check for
    /// either source can run, rather than relying on timing.
    #[test]
    fn register_all_skips_a_source_deleted_before_it_reaches_it() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("focus12");
        data::fixtures::generate_focus12_fixture(&data_dir).unwrap();
        let settings_path = dir.path().join("settings.toml");
        let secrets = Arc::new(MemorySecretStore::default());
        let svc = Arc::new(CostlyticsService::new(AppConfig::default(), Some(settings_path), secrets));
        let fixture_path = data_dir.to_str().unwrap().to_string();

        svc.save_source(SaveSourceRequest { source: local("a", &fixture_path), secret: None, is_new: true })
            .unwrap();
        svc.save_source(SaveSourceRequest { source: local("b", &fixture_path), secret: None, is_new: true })
            .unwrap();

        // Hold `mutation` so `register_all`'s per-source checks (for both
        // "a" and "b") cannot start until the deletion below has already
        // landed.
        let guard = svc.mutation.lock().unwrap();

        let register_handle = {
            let svc = svc.clone();
            std::thread::spawn(move || svc.register_all())
        };

        // Delete "b" directly (bypassing `delete_source`, which would also
        // need `mutation` and thus deadlock against the guard above) while
        // `register_all`'s thread is blocked waiting for the lock.
        {
            let mut cfg = svc.config.write().unwrap();
            cfg.sources.retain(|s| s.id != "b");
        }
        svc.registry.remove("b");

        drop(guard);
        register_handle.join().unwrap();

        assert!(svc.registry.diagnostics().iter().all(|d| d.id != "b"));
        assert!(svc.sources().sources.iter().all(|s| s.id != "b"));
        // "a" was untouched and still gets registered normally.
        assert!(svc.registry.diagnostics().iter().any(|d| d.id == "a"
            && matches!(d.status, crate::registry::SourceStatus::Registered { .. })));
    }

    #[test]
    fn save_cost_guard_persists_and_validates() {
        let f = fixture();
        let guard = CostGuardConfig { soft_limit_usd: 0.5, hard_limit_usd: 2.0, ..Default::default() };
        f.svc.save_cost_guard(guard.clone()).unwrap();
        assert_eq!(f.svc.settings().cost_guard, guard);
        assert_eq!(AppConfig::load(f.settings_path.to_str().unwrap()).unwrap().cost_guard, guard);

        let inverted = CostGuardConfig { soft_limit_usd: 3.0, hard_limit_usd: 1.0, ..Default::default() };
        assert!(f.svc.save_cost_guard(inverted).is_err());
        let negative = CostGuardConfig { egress_usd_per_gb: -1.0, ..Default::default() };
        assert!(f.svc.save_cost_guard(negative).is_err());
        assert_eq!(f.svc.settings().cost_guard, guard, "rejected saves leave the settings untouched");
    }
}
