use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

pub use crate::object_store::YearMonth;

/// Errors that can occur when parsing a manifest file.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("Failed to parse manifest JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("IO error reading manifest: {0}")]
    Io(#[from] std::io::Error),
}

/// Represents one AWS Data Export execution's manifest.
#[derive(Debug, Clone)]
pub struct DatasetPartition {
    pub billing_period: YearMonth,
    pub manifest_uri: String,
    pub files: Vec<String>, // absolute URIs/paths to Parquet/CSV files
    pub updated_at: DateTime<Utc>,
}

/// The JSON structure of Manifest.json (AWS Data Exports format).
/// Handles both `dataFiles` (array of strings) and `files` (array of objects).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestJson {
    /// List of data file paths relative to the manifest, OR absolute S3 URIs.
    #[serde(default)]
    pub data_files: Vec<String>,
    /// Alternative field used by different export types (array of objects).
    #[serde(default)]
    pub files: Vec<ManifestFile>,
    pub billing_period: Option<ManifestBillingPeriod>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    pub key: Option<String>,
    pub data_file: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestBillingPeriod {
    pub start: Option<String>,
    pub end: Option<String>,
}

/// Parse a `Manifest.json` file into a [`DatasetPartition`].
///
/// - `billing_period`: the `YearMonth` this manifest belongs to.
/// - `manifest_uri`: the path/URI of the manifest file itself.
/// - `content`: raw bytes of the manifest JSON.
/// - `base_uri`: directory prefix to prepend to relative file paths.
pub fn parse_manifest(
    billing_period: YearMonth,
    manifest_uri: &str,
    content: &[u8],
    base_uri: &str,
) -> Result<DatasetPartition, ManifestError> {
    let manifest: ManifestJson = serde_json::from_slice(content)?;

    // Collect all file paths from either `dataFiles` or `files[].key/dataFile`
    let mut raw_paths: Vec<String> = Vec::new();

    for path in &manifest.data_files {
        raw_paths.push(path.clone());
    }

    for file in &manifest.files {
        if let Some(path) = file.key.as_deref().or(file.data_file.as_deref()) {
            raw_paths.push(path.to_string());
        }
    }

    // Resolve relative paths to absolute using base_uri
    let files = raw_paths
        .into_iter()
        .map(|p| resolve_path(&p, base_uri))
        .collect();

    Ok(DatasetPartition {
        billing_period,
        manifest_uri: manifest_uri.to_string(),
        files,
        updated_at: Utc::now(),
    })
}

/// Resolve a file path: if it's already absolute (starts with `/`, a drive letter,
/// or a URI scheme like `s3://`), return as-is. Otherwise join with `base_uri`.
fn resolve_path(path: &str, base_uri: &str) -> String {
    // Absolute S3 or other URI scheme
    if path.contains("://") {
        return path.to_string();
    }
    // Absolute POSIX path
    if path.starts_with('/') {
        return path.to_string();
    }
    // Absolute Windows path (e.g. C:\... or C:/...)
    if path.len() >= 3 {
        let bytes = path.as_bytes();
        if bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
            return path.to_string();
        }
    }
    // Relative: join with base_uri
    let base = base_uri.trim_end_matches('/').trim_end_matches('\\');
    format!("{}/{}", base, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_data_files_field() {
        let json = serde_json::json!({
            "dataFiles": [
                "data/export-00001.parquet",
                "data/export-00002.parquet"
            ]
        });
        let content = serde_json::to_vec(&json).unwrap();
        let partition = parse_manifest(
            YearMonth::new(2026, 8),
            "/base/BILLING_PERIOD=2026-08/Manifest.json",
            &content,
            "/base/BILLING_PERIOD=2026-08",
        )
        .unwrap();

        assert_eq!(partition.billing_period, YearMonth::new(2026, 8));
        assert_eq!(partition.files.len(), 2);
        assert_eq!(
            partition.files[0],
            "/base/BILLING_PERIOD=2026-08/data/export-00001.parquet"
        );
        assert_eq!(
            partition.files[1],
            "/base/BILLING_PERIOD=2026-08/data/export-00002.parquet"
        );
    }

    #[test]
    fn parse_manifest_files_with_key_field() {
        let json = serde_json::json!({
            "files": [
                { "key": "data/part-0001.parquet" },
                { "key": "data/part-0002.parquet" }
            ]
        });
        let content = serde_json::to_vec(&json).unwrap();
        let partition = parse_manifest(
            YearMonth::new(2026, 9),
            "/base/BILLING_PERIOD=2026-09/Manifest.json",
            &content,
            "/base/BILLING_PERIOD=2026-09",
        )
        .unwrap();

        assert_eq!(partition.files.len(), 2);
        assert!(partition.files[0].ends_with("part-0001.parquet"));
    }

    #[test]
    fn parse_manifest_files_with_data_file_field() {
        let json = serde_json::json!({
            "files": [
                { "dataFile": "exports/part-0001.parquet" }
            ]
        });
        let content = serde_json::to_vec(&json).unwrap();
        let partition = parse_manifest(
            YearMonth::new(2026, 8),
            "/base/BILLING_PERIOD=2026-08/Manifest.json",
            &content,
            "/base/BILLING_PERIOD=2026-08",
        )
        .unwrap();

        assert_eq!(partition.files.len(), 1);
        assert!(partition.files[0].ends_with("part-0001.parquet"));
    }

    #[test]
    fn parse_manifest_absolute_s3_uris_not_modified() {
        let json = serde_json::json!({
            "dataFiles": [
                "s3://my-bucket/exports/2026-08/data.parquet"
            ]
        });
        let content = serde_json::to_vec(&json).unwrap();
        let partition = parse_manifest(
            YearMonth::new(2026, 8),
            "s3://my-bucket/exports/2026-08/Manifest.json",
            &content,
            "s3://my-bucket/exports/2026-08",
        )
        .unwrap();

        assert_eq!(
            partition.files[0],
            "s3://my-bucket/exports/2026-08/data.parquet"
        );
    }

    #[test]
    fn parse_manifest_empty_files() {
        let json = serde_json::json!({});
        let content = serde_json::to_vec(&json).unwrap();
        let partition = parse_manifest(
            YearMonth::new(2026, 8),
            "/base/BILLING_PERIOD=2026-08/Manifest.json",
            &content,
            "/base/BILLING_PERIOD=2026-08",
        )
        .unwrap();
        assert!(partition.files.is_empty());
    }

    #[test]
    fn parse_manifest_invalid_json_returns_error() {
        let result = parse_manifest(
            YearMonth::new(2026, 8),
            "/some/Manifest.json",
            b"not valid json {{{",
            "/some",
        );
        assert!(result.is_err());
    }

    #[test]
    fn resolve_path_relative() {
        assert_eq!(
            resolve_path("data/file.parquet", "/base/dir"),
            "/base/dir/data/file.parquet"
        );
    }

    #[test]
    fn resolve_path_absolute_posix() {
        assert_eq!(
            resolve_path("/absolute/path/file.parquet", "/base/dir"),
            "/absolute/path/file.parquet"
        );
    }

    #[test]
    fn resolve_path_absolute_s3() {
        assert_eq!(
            resolve_path("s3://bucket/key/file.parquet", "/base/dir"),
            "s3://bucket/key/file.parquet"
        );
    }
}
