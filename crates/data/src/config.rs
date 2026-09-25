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
}
