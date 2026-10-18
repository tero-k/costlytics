use domain::anomaly::{Rules, ScopeKind};
use domain::cost::CostMetric;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("Invalid port in COSTLYTICS_PORT: {0}")]
    InvalidPort(#[from] std::num::ParseIntError),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 3000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    #[default]
    Auto,
    Cur2,
    Focus10,
    Focus12,
}

/// How DuckDB authenticates to S3 for an `s3://` source. Ignored for local
/// sources. The access-key *secret* is never part of this type — it lives
/// in the OS keychain (see `service::secrets`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum S3AuthConfig {
    /// AWS credential chain: env vars, `~/.aws` config/SSO, instance role.
    /// `DataSource::aws_profile` optionally pins a named profile.
    #[default]
    CredentialChain,
    /// Static access key; the secret half is stored in the OS keychain.
    AccessKey { key_id: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct DataSource {
    pub id: String,
    pub name: String,
    /// Either `s3://bucket/prefix/` or a local filesystem path.
    pub s3_uri: String,
    #[serde(default)]
    pub source_type: SourceType,
    pub aws_region: Option<String>,
    pub aws_profile: Option<String>,
    pub role_arn: Option<String>,
    #[serde(default)]
    pub auth: S3AuthConfig,
}

impl DataSource {
    /// Returns true when the URI points at an S3 location.
    pub fn is_s3(&self) -> bool {
        self.s3_uri.starts_with("s3://")
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub sources: Vec<DataSource>,
    #[serde(default)]
    pub cost_guard: CostGuardConfig,
    #[serde(default)]
    pub anomaly: AnomalyConfig,
}

/// Warns before a date range pulls an expensive amount of data from S3.
/// Limits are per page load; rates default to S3 internet egress and
/// Standard-tier GET pricing (us-east-1) and ignore the free allowance.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct CostGuardConfig {
    pub enabled: bool,
    /// Above this, the page loads and shows a warning banner.
    pub soft_limit_usd: f64,
    /// Above this, the page asks for confirmation before loading.
    pub hard_limit_usd: f64,
    pub egress_usd_per_gb: f64,
    pub get_usd_per_1000: f64,
}

impl Default for CostGuardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            soft_limit_usd: 0.10,
            hard_limit_usd: 1.00,
            egress_usd_per_gb: 0.09,
            get_usd_per_1000: 0.0004,
        }
    }
}

/// Spend anomaly detection rules (`[anomaly]`). Per-scope overrides replace
/// individual rule fields; metric, lookback, settling and interval are global.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AnomalyConfig {
    pub enabled: bool,
    pub metric: CostMetric,
    pub lookback_days: u32,
    /// Newest days of data AWS is still revising; never evaluated.
    pub settling_days: u32,
    pub mad_multiplier: f64,
    /// Per day.
    pub min_delta_usd: f64,
    pub min_delta_pct: f64,
    /// 0 = rule off.
    pub min_monthly_impact_usd: f64,
    pub include_drops: bool,
    pub severity_medium_usd: f64,
    pub severity_high_usd: f64,
    pub check_interval_hours: u32,
    /// Kept last: TOML needs plain values before arrays of tables.
    pub overrides: Vec<AnomalyOverride>,
}

impl Default for AnomalyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            metric: CostMetric::Amortized,
            lookback_days: 28,
            settling_days: 2,
            mad_multiplier: 3.0,
            min_delta_usd: 20.0,
            min_delta_pct: 0.25,
            min_monthly_impact_usd: 0.0,
            include_drops: false,
            severity_medium_usd: 500.0,
            severity_high_usd: 2000.0,
            check_interval_hours: 6,
            overrides: Vec::new(),
        }
    }
}

/// `[[anomaly.overrides]]`: rule fields for one service or account; `None` inherits.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnomalyOverride {
    pub scope: ScopeKind,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mad_multiplier: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_delta_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_delta_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_monthly_impact_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_drops: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity_medium_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity_high_usd: Option<f64>,
}

fn check_non_negative(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be a non-negative number"))
    }
}

/// Checks one resolved rule set; shared by the globals and each override.
fn check_rules(r: &Rules) -> Result<(), String> {
    check_non_negative("mad_multiplier", r.mad_multiplier)?;
    check_non_negative("min_delta_usd", r.min_delta_usd)?;
    check_non_negative("min_delta_pct", r.min_delta_pct)?;
    check_non_negative("min_monthly_impact_usd", r.min_monthly_impact_usd)?;
    check_non_negative("severity_medium_usd", r.severity_medium_usd)?;
    check_non_negative("severity_high_usd", r.severity_high_usd)?;
    if r.mad_multiplier <= 0.0 {
        return Err("mad_multiplier must be greater than 0".into());
    }
    if r.severity_high_usd < r.severity_medium_usd {
        return Err("severity_high_usd must be at least severity_medium_usd".into());
    }
    Ok(())
}

impl AnomalyConfig {
    fn global_rules(&self) -> Rules {
        Rules {
            lookback_days: self.lookback_days,
            mad_multiplier: self.mad_multiplier,
            min_delta_usd: self.min_delta_usd,
            min_delta_pct: self.min_delta_pct,
            min_monthly_impact_usd: self.min_monthly_impact_usd,
            include_drops: self.include_drops,
            severity_medium_usd: self.severity_medium_usd,
            severity_high_usd: self.severity_high_usd,
        }
    }

    /// The rules for one series: globals with that scope+key's override applied.
    pub fn rules_for(&self, scope: ScopeKind, key: &str) -> Rules {
        let mut r = self.global_rules();
        if let Some(o) = self
            .overrides
            .iter()
            .find(|o| o.scope == scope && o.key.trim() == key)
        {
            if let Some(v) = o.mad_multiplier {
                r.mad_multiplier = v;
            }
            if let Some(v) = o.min_delta_usd {
                r.min_delta_usd = v;
            }
            if let Some(v) = o.min_delta_pct {
                r.min_delta_pct = v;
            }
            if let Some(v) = o.min_monthly_impact_usd {
                r.min_monthly_impact_usd = v;
            }
            if let Some(v) = o.include_drops {
                r.include_drops = v;
            }
            if let Some(v) = o.severity_medium_usd {
                r.severity_medium_usd = v;
            }
            if let Some(v) = o.severity_high_usd {
                r.severity_high_usd = v;
            }
        }
        r
    }

    /// Human-readable error for the first invalid value; used on save and before each check.
    pub fn validate(&self) -> Result<(), String> {
        check_rules(&self.global_rules())?;
        if !(14..=90).contains(&self.lookback_days) {
            return Err("lookback_days must be between 14 and 90".into());
        }
        if self.settling_days > 7 {
            return Err("settling_days must be between 0 and 7".into());
        }
        if self.check_interval_hours < 1 {
            return Err("check_interval_hours must be at least 1".into());
        }
        let mut seen = std::collections::HashSet::new();
        for o in &self.overrides {
            let key = o.key.trim();
            if key.is_empty() {
                return Err("override key must not be empty".into());
            }
            if !seen.insert((o.scope, key.to_string())) {
                return Err(format!(
                    "duplicate override for {} '{key}'",
                    o.scope.as_str()
                ));
            }
            check_rules(&self.rules_for(o.scope, key))
                .map_err(|e| format!("override {} '{key}': {e}", o.scope.as_str()))?;
        }
        Ok(())
    }
}

impl AppConfig {
    /// Load config from TOML file, then overlay `COSTLYTICS_*` environment variables.
    /// A missing file is treated as an empty config (all defaults).
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(ConfigError::Io(e)),
        };
        let mut cfg: AppConfig = toml::from_str(&text)?;
        // Environment overrides
        if let Ok(port) = std::env::var("COSTLYTICS_PORT") {
            cfg.server.port = port.parse()?;
        }
        if let Ok(host) = std::env::var("COSTLYTICS_HOST") {
            cfg.server.host = host;
        }
        Ok(cfg)
    }

    /// Write this config as TOML to `path` (used by the desktop Settings tab).
    ///
    /// Writes to a sibling `.toml.tmp` file first, then renames it over
    /// `path`, so a crash or power loss mid-write can never leave `path`
    /// truncated or half-written — the rename is atomic on both POSIX and
    /// Windows (on Windows, `rename` replaces an existing destination file
    /// rather than failing, matching POSIX `rename(2)`'s behavior here).
    pub fn save(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self)?;
        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, text)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;
    use tempfile::NamedTempFile;

    /// Mutex to serialize tests that mutate process-wide environment variables.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_is_s3_true() {
        let ds = DataSource {
            id: "test".into(),
            name: "Test".into(),
            s3_uri: "s3://my-bucket/prefix/".into(),
            source_type: SourceType::Auto,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
        };
        assert!(ds.is_s3());
    }

    #[test]
    fn test_is_s3_false() {
        let ds = DataSource {
            id: "test".into(),
            name: "Test".into(),
            s3_uri: "/local/path/to/data".into(),
            source_type: SourceType::Auto,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
        };
        assert!(!ds.is_s3());
    }

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure env vars are not set before testing defaults
        std::env::remove_var("COSTLYTICS_PORT");
        std::env::remove_var("COSTLYTICS_HOST");
        let cfg = AppConfig::load("/nonexistent/path/config.toml").unwrap();
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 3000);
        assert!(cfg.sources.is_empty());
    }

    #[test]
    fn test_load_from_toml_file() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("COSTLYTICS_PORT");
        std::env::remove_var("COSTLYTICS_HOST");
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[server]
host = "0.0.0.0"
port = 8080

[[sources]]
id = "main"
name = "Main"
s3_uri = "s3://my-bucket/cur/"
aws_region = "eu-west-1"
"#
        )
        .unwrap();

        let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.sources.len(), 1);
        assert!(cfg.sources[0].is_s3());
    }

    #[test]
    fn test_load_env_override() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // Use a temp file with defaults, then override via env
        let f = NamedTempFile::new().unwrap();
        std::env::set_var("COSTLYTICS_PORT", "9999");
        std::env::set_var("COSTLYTICS_HOST", "192.168.1.1");
        let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
        std::env::remove_var("COSTLYTICS_PORT");
        std::env::remove_var("COSTLYTICS_HOST");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.server.host, "192.168.1.1");
    }

    #[test]
    fn auth_defaults_to_credential_chain_when_absent() {
        let cfg: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "a"
name = "A"
s3_uri = "s3://b/p/"
"#,
        )
        .unwrap();
        assert_eq!(cfg.sources[0].auth, S3AuthConfig::CredentialChain);
    }

    #[test]
    fn save_then_load_round_trips_access_key_auth() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("COSTLYTICS_PORT");
        std::env::remove_var("COSTLYTICS_HOST");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let cfg = AppConfig {
            server: ServerConfig::default(),
            cost_guard: Default::default(),
            anomaly: Default::default(),
            sources: vec![DataSource {
                id: "prod".into(),
                name: "Prod".into(),
                s3_uri: "s3://bucket/exports/data".into(),
                aws_region: Some("eu-north-1".into()),
                auth: S3AuthConfig::AccessKey { key_id: "AKIAEXAMPLE".into() },
                ..Default::default()
            }],
        };
        cfg.save(&path).unwrap();
        let loaded = AppConfig::load(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.sources, cfg.sources);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.to_lowercase().contains("secret"), "settings file must never hold a secret: {text}");
    }

    #[test]
    fn anomaly_defaults_match_the_spec() {
        let cfg: AppConfig = toml::from_str("").unwrap();
        let a = cfg.anomaly;
        assert!(a.enabled);
        assert_eq!(a.metric, domain::cost::CostMetric::Amortized);
        assert_eq!((a.lookback_days, a.settling_days, a.check_interval_hours), (28, 2, 6));
        assert_eq!((a.mad_multiplier, a.min_delta_usd, a.min_delta_pct), (3.0, 20.0, 0.25));
        assert_eq!(a.min_monthly_impact_usd, 0.0);
        assert!(!a.include_drops, "drops are opt-in");
        assert_eq!((a.severity_medium_usd, a.severity_high_usd), (500.0, 2000.0));
        assert!(a.overrides.is_empty());
        assert_eq!(a.validate(), Ok(()));
    }

    #[test]
    fn anomaly_overrides_round_trip_and_apply() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("COSTLYTICS_PORT");
        std::env::remove_var("COSTLYTICS_HOST");
        let text = r#"
[anomaly]
min_delta_usd = 30.0

[[anomaly.overrides]]
scope = "account"
key = "123456789012"
min_delta_usd = 100.0
include_drops = true
"#;
        let cfg: AppConfig = toml::from_str(text).unwrap();
        let rules = cfg.anomaly.rules_for(domain::anomaly::ScopeKind::Account, "123456789012");
        assert_eq!((rules.min_delta_usd, rules.include_drops), (100.0, true));
        assert_eq!(rules.min_delta_pct, 0.25, "unset override fields inherit");
        let other = cfg.anomaly.rules_for(domain::anomaly::ScopeKind::Service, "123456789012");
        assert_eq!(other.min_delta_usd, 30.0, "override only matches its own scope");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        cfg.save(&path).unwrap();
        let loaded = AppConfig::load(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.anomaly, cfg.anomaly);
    }

    #[test]
    fn anomaly_validation_rejects_bad_values() {
        let ok = AnomalyConfig::default();
        let cases: Vec<(AnomalyConfig, &str)> = vec![
            (AnomalyConfig { lookback_days: 13, ..ok.clone() }, "lookback_days must be between 14 and 90"),
            (AnomalyConfig { lookback_days: 0, ..ok.clone() }, "lookback_days must be between 14 and 90"),
            (AnomalyConfig { settling_days: 8, ..ok.clone() }, "settling_days must be between 0 and 7"),
            (AnomalyConfig { mad_multiplier: 0.0, ..ok.clone() }, "mad_multiplier must be greater than 0"),
            (AnomalyConfig { min_delta_usd: -1.0, ..ok.clone() }, "min_delta_usd must be a non-negative number"),
            (AnomalyConfig { min_delta_pct: f64::NAN, ..ok.clone() }, "min_delta_pct must be a non-negative number"),
            (AnomalyConfig { severity_high_usd: 100.0, ..ok.clone() }, "severity_high_usd must be at least severity_medium_usd"),
            (AnomalyConfig { check_interval_hours: 0, ..ok.clone() }, "check_interval_hours must be at least 1"),
        ];
        for (cfg, message) in cases {
            assert_eq!(cfg.validate(), Err(message.to_string()));
        }

        let ovr = |key: &str| AnomalyOverride {
            scope: domain::anomaly::ScopeKind::Service,
            key: key.into(),
            mad_multiplier: None,
            min_delta_usd: None,
            min_delta_pct: None,
            min_monthly_impact_usd: None,
            include_drops: None,
            severity_medium_usd: None,
            severity_high_usd: None,
        };
        let blank = AnomalyConfig { overrides: vec![ovr("  ")], ..ok.clone() };
        assert_eq!(blank.validate(), Err("override key must not be empty".to_string()));
        let dup = AnomalyConfig { overrides: vec![ovr("EC2"), ovr("EC2")], ..ok.clone() };
        assert_eq!(dup.validate(), Err("duplicate override for service 'EC2'".to_string()));
        let mut neg = ovr("EC2");
        neg.min_delta_usd = Some(-5.0);
        let neg = AnomalyConfig { overrides: vec![neg], ..ok.clone() };
        assert_eq!(neg.validate(), Err("override service 'EC2': min_delta_usd must be a non-negative number".to_string()));
        let mut inverted = ovr("EC2");
        inverted.severity_high_usd = Some(10.0);
        let inverted = AnomalyConfig { overrides: vec![inverted], ..ok };
        assert_eq!(
            inverted.validate(),
            Err("override service 'EC2': severity_high_usd must be at least severity_medium_usd".to_string())
        );
    }
}
