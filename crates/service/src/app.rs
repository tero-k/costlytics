use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use data::config::{AppConfig, DataSource, S3AuthConfig};
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Deserialize)]
pub struct SaveSourceRequest {
    pub source: DataSource,
    /// New access-key secret; `None`/empty keeps the stored one.
    #[serde(default)]
    pub secret: Option<String>,
    pub is_new: bool,
}

#[derive(Debug, Deserialize)]
pub struct TestSourceRequest {
    pub source: DataSource,
    #[serde(default)]
    pub secret: Option<String>,
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
        Self { registry, config: RwLock::new(config), settings_path, secrets, mutation: Mutex::new(()) }
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

    /// Blocking: (re)registers one source, updating the registry.
    fn register_one(&self, source: &DataSource, secret: Option<&str>) {
        self.registry.mark_pending(source);
        let result = register_source(source, secret);
        match &result {
            Ok(r) => tracing::info!(source_id = %source.id, file_count = r.file_count, format = %r.detected_format, "source registered"),
            Err(reason) => tracing::warn!(source_id = %source.id, %reason, "source skipped"),
        }
        self.registry.set_result(&source.id, result);
    }

    fn entry(&self, id: &str) -> Result<SourceEntry, ServiceError> {
        entry_for(&self.registry, id).ok_or_else(|| ServiceError::not_found(format!("unknown source '{id}'")))
    }

    /// Blocking: registers every configured source, in order.
    pub fn register_all(&self) {
        for source in self.config().sources {
            let secret = match source.auth {
                S3AuthConfig::AccessKey { .. } => self.stored_secret(&source.id),
                S3AuthConfig::CredentialChain => None,
            };
            self.register_one(&source, secret.as_deref());
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
        SettingsResponse { sources }
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
        drop(_guard);

        self.register_one(&source, secret.as_deref());
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
        drop(_guard);
        if let Err(e) = self.secrets.delete(&req.id) {
            tracing::warn!(source_id = %req.id, error = %e, "could not delete secret from keychain");
        }
        self.registry.remove(&req.id);
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
        let source = self.find(&req.id).ok_or_else(|| ServiceError::not_found(format!("unknown source '{}'", req.id)))?;
        let secret = match source.auth {
            S3AuthConfig::AccessKey { .. } => self.stored_secret(&source.id),
            S3AuthConfig::CredentialChain => None,
        };
        self.register_one(&source, secret.as_deref());
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
}
