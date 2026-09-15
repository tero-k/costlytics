use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    #[default]
    Auto,
    Cur2,
    Focus10,
    Focus12,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
}

impl DataSource {
    /// Returns true when the URI points at an S3 location.
    pub fn is_s3(&self) -> bool {
        self.s3_uri.starts_with("s3://")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub sources: Vec<DataSource>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            sources: Vec::new(),
        }
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
}
