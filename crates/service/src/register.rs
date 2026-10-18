use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Utc};
use data::adapters::{cur2, focus10, focus12};
use data::config::{DataSource, S3AuthConfig};
use data::duckdb_pool::{self, S3Auth};
use data::object_store::{DuckDbObjectStore, LocalObjectStore, YearMonth};
use data::queries::summary::{CostRepository, DuckDbCostRepository};
use data::scan_index::{self, ScanIndex};
use data::schema_detection::{self, DetectedSchema};

/// A successfully registered source: a queryable repository plus what
/// discovery/detection found.
pub struct Registered {
    pub repo: Arc<dyn CostRepository>,
    pub detected_format: String,
    pub file_count: usize,
    pub billing_periods: Vec<String>,
    /// Whether queries read over the network (an `s3://` source).
    pub remote: bool,
    /// Row-group index for cost estimates; `None` if the source is empty or
    /// the footers could not be indexed (estimates then report "unknown").
    pub scan_index: Option<Arc<ScanIndex>>,
    /// Identifies the exact data registered: file list + footer index. Changes
    /// when AWS delivers or rewrites files.
    pub fingerprint: String,
}

impl std::fmt::Debug for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registered")
            .field("detected_format", &self.detected_format)
            .field("file_count", &self.file_count)
            .field("billing_periods", &self.billing_periods)
            .field("remote", &self.remote)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// Default date range for partition discovery: the last 5 years through the
/// end of next month (moved unchanged from `crates/api/src/lib.rs`).
pub fn default_discovery_range() -> (NaiveDate, NaiveDate) {
    let today = Utc::now().date_naive();
    let start = NaiveDate::from_ymd_opt(today.year() - 5, today.month(), 1).unwrap_or(today);
    let current_ym = YearMonth::new(today.year(), today.month());
    let next_ym = if current_ym.month == 12 {
        YearMonth::new(current_ym.year + 1, 1)
    } else {
        YearMonth::new(current_ym.year, current_ym.month + 1)
    };
    (start, next_ym.end_date_exclusive())
}

/// Replaces every occurrence of `secret` in `text` with `***`, so an error
/// string that happens to embed a raw access-key secret (e.g. from a DuckDB
/// error message that echoes the credentials it was given) never reaches a
/// log line or a skip reason shown in Settings. A no-op if `secret` is empty
/// (an empty needle would match everywhere and corrupt the text).
fn scrub_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "***")
}

fn s3_auth(source: &DataSource, secret: Option<&str>) -> Result<S3Auth, String> {
    match &source.auth {
        S3AuthConfig::CredentialChain => Ok(S3Auth::CredentialChain {
            profile: source.aws_profile.clone(),
            region: source.aws_region.clone(),
        }),
        S3AuthConfig::AccessKey { key_id } => {
            let secret = secret.ok_or("access key secret is not set; re-enter it in Settings")?;
            Ok(S3Auth::AccessKey {
                key_id: key_id.clone(),
                secret: secret.to_string(),
                region: source.aws_region.clone(),
            })
        }
    }
}

/// Stable FNV-1a 64 hash of the sorted file list and the footer index, so a
/// file rewritten in place under the same name still changes it. Persisted
/// in `alerts.json`, hence a fixed algorithm rather than `DefaultHasher`.
pub fn data_fingerprint(files: &[String], index: Option<&ScanIndex>) -> String {
    let mut hasher = Fnv1a::default();
    let mut feed = |bytes: &[u8]| hasher.feed(bytes);
    let mut sorted: Vec<&String> = files.iter().collect();
    sorted.sort();
    for f in sorted {
        feed(f.as_bytes());
        feed(&[0]);
    }
    if let Some(index) = index {
        for e in &index.entries {
            feed(format!("{}|{:?}|{:?}|{}|{};", e.file, e.min_ts, e.max_ts, e.bytes, e.column_chunks).as_bytes());
        }
    }
    hasher.hex()
}

/// FNV-1a 64: a fixed hash for fingerprints persisted across runs.
pub(crate) struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(PRIME);
        }
    }

    pub(crate) fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Discover, schema-detect and register one source's `normalized_cost` view
/// in a fresh in-memory DuckDB pool. Blocking (file/S3 I/O). `secret` is the
/// access-key secret for `S3AuthConfig::AccessKey` sources.
///
/// An existing-but-empty location registers with `detected_format: "none"`
/// and `file_count: 0` (it answers queries with zero rows), matching the
/// pre-desktop `build_app` behavior.
pub fn register_source(source: &DataSource, secret: Option<&str>) -> Result<Registered, String> {
    // Resolve credentials first so a missing secret fails fast, offline.
    let auth = if source.is_s3() { Some(s3_auth(source, secret)?) } else { None };

    let pool = duckdb_pool::build_pool().map_err(|e| format!("failed to build DuckDB pool: {e}"))?;
    let conn = pool.get().map_err(|e| {
        format!("failed to get pooled connection for schema detection/view registration: {e}")
    })?;
    conn.execute_batch("LOAD parquet;")
        .map_err(|e| format!("failed to load parquet extension: {e}"))?;

    let (range_start, range_end) = default_discovery_range();
    let partitions = match &auth {
        Some(auth) => {
            duckdb_pool::init_connection(&conn, auth).map_err(|e| {
                let mut msg = e.to_string();
                if let S3Auth::AccessKey { secret, .. } = auth {
                    msg = scrub_secret(&msg, secret);
                }
                format!("failed to initialise S3 access (httpfs/aws extensions, credentials): {msg}")
            })?;
            let store = DuckDbObjectStore::new(pool.clone());
            data::discovery::discover_partitions(&store, &source.s3_uri, range_start, range_end)
        }
        None => data::discovery::discover_partitions(&LocalObjectStore, &source.s3_uri, range_start, range_end),
    }
    .map_err(|e| format!("partition discovery failed: {e}"))?;

    let billing_periods: Vec<String> =
        partitions.iter().map(|p| p.billing_period.to_ym_string()).collect();
    let files: Vec<String> = partitions.into_iter().flat_map(|p| p.files).collect();

    let mut detected_format = "none";
    let mut scan_index = None;
    if !files.is_empty() {
        let (register, columns) = match schema_detection::detect_schema(&conn, &files[0]) {
            Ok(DetectedSchema::Focus12) => {
                detected_format = "focus12";
                (focus12::register_view(&conn, &files), &scan_index::FOCUS_COLUMNS)
            }
            Ok(DetectedSchema::Cur2) => {
                detected_format = "cur2";
                (cur2::register_view(&conn, &files), &scan_index::CUR2_COLUMNS)
            }
            Ok(DetectedSchema::Focus10) => {
                detected_format = "focus10";
                (focus10::register_view(&conn, &files), &scan_index::FOCUS_COLUMNS)
            }
            Ok(_) => return Err("detected schema is not FOCUS 1.0/1.2 or CUR 2.0".to_string()),
            Err(e) => return Err(format!("schema detection failed: {e}")),
        };
        register.map_err(|e| format!("failed to register view: {e}"))?;
        // Best effort: a source without an index still answers queries.
        scan_index = match ScanIndex::build(&conn, &files, columns) {
            Ok(index) => Some(Arc::new(index)),
            Err(e) => {
                let mut error = e.to_string();
                if let Some(S3Auth::AccessKey { secret, .. }) = &auth {
                    error = scrub_secret(&error, secret);
                }
                tracing::warn!(source = %source.id, %error, "failed to index Parquet footers for cost estimates");
                None
            }
        };
    }
    drop(conn);

    let fingerprint = data_fingerprint(&files, scan_index.as_deref());
    Ok(Registered {
        repo: Arc::new(DuckDbCostRepository::new(pool)),
        detected_format: detected_format.to_string(),
        file_count: files.len(),
        billing_periods,
        remote: source.is_s3(),
        scan_index,
        fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use data::config::{S3AuthConfig, DataSource};

    fn local(dir: &std::path::Path) -> DataSource {
        DataSource { id: "t".into(), name: "T".into(), s3_uri: dir.to_str().unwrap().into(), ..Default::default() }
    }

    #[test]
    fn fingerprint_changes_with_files_or_footers_but_is_stable() {
        use data::scan_index::{RowGroupEntry, ScanIndex};
        let files = vec!["a.parquet".to_string(), "b.parquet".to_string()];
        let index = |bytes| ScanIndex {
            entries: vec![RowGroupEntry { file: 0, min_ts: None, max_ts: None, bytes, column_chunks: 3 }],
            file_count: 2,
        };
        let base = data_fingerprint(&files, Some(&index(100)));
        assert_eq!(base.len(), 16);
        assert_eq!(base, data_fingerprint(&files, Some(&index(100))));
        assert_ne!(base, data_fingerprint(&files, Some(&index(101))), "a rewritten file changes its footer");
        assert_ne!(base, data_fingerprint(&files[..1], Some(&index(100))));
        let reversed: Vec<String> = files.iter().rev().cloned().collect();
        assert_eq!(base, data_fingerprint(&reversed, Some(&index(100))), "file order doesn't matter");
    }

    #[test]
    fn scrub_secret_redacts_every_occurrence() {
        assert_eq!(scrub_secret("auth failed for TOPSECRET at TOPSECRET", "TOPSECRET"), "auth failed for *** at ***");
        assert_eq!(scrub_secret("no secret here", "TOPSECRET"), "no secret here");
        // Guard against an empty secret turning into a no-op match-everywhere.
        assert_eq!(scrub_secret("unchanged", ""), "unchanged");
    }

    #[test]
    fn registers_local_focus12_fixture() {
        let dir = tempfile::tempdir().unwrap();
        data::fixtures::generate_focus12_fixture(dir.path()).unwrap();
        let r = register_source(&local(dir.path()), None).unwrap();
        assert_eq!(r.detected_format, "focus12");
        assert!(r.file_count > 0);
        assert!(r.billing_periods.contains(&"2026-08".to_string()));
    }

    /// Exports that write `billing_period=YYYY-MM` in lower case must register
    /// every month, not just upper-case ones.
    #[test]
    fn registers_lowercase_billing_period_dirs() {
        let dir = tempfile::tempdir().unwrap();
        data::fixtures::generate_focus12_fixture(dir.path()).unwrap();
        for ym in ["2026-08", "2026-09"] {
            std::fs::rename(
                dir.path().join(format!("BILLING_PERIOD={ym}")),
                dir.path().join(format!("billing_period={ym}")),
            )
            .unwrap();
        }
        let r = register_source(&local(dir.path()), None).unwrap();
        assert_eq!(r.detected_format, "focus12");
        assert_eq!(r.billing_periods, vec!["2026-08".to_string(), "2026-09".to_string()]);
        assert_eq!(r.file_count, 2);
    }

    #[test]
    fn registers_local_cur2_fixture() {
        let dir = tempfile::tempdir().unwrap();
        data::fixtures::generate_cur2_fixture(dir.path()).unwrap();
        assert_eq!(register_source(&local(dir.path()), None).unwrap().detected_format, "cur2");
    }

    #[test]
    fn empty_local_dir_registers_with_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let r = register_source(&local(dir.path()), None).unwrap();
        assert_eq!((r.detected_format.as_str(), r.file_count), ("none", 0));
    }

    #[test]
    fn non_parquet_data_is_skipped_with_reason() {
        let dir = tempfile::tempdir().unwrap();
        let pd = dir.path().join("BILLING_PERIOD=2026-08");
        std::fs::create_dir(&pd).unwrap();
        std::fs::write(pd.join("Manifest.json"), br#"{"dataFiles":["data.parquet"]}"#).unwrap();
        std::fs::write(pd.join("data.parquet"), b"not parquet").unwrap();
        let reason = register_source(&local(dir.path()), None).unwrap_err();
        assert!(reason.starts_with("schema detection failed"), "{reason}");
    }

    #[test]
    fn access_key_without_secret_is_rejected_before_any_network() {
        let s = DataSource {
            id: "s".into(),
            name: "S".into(),
            s3_uri: "s3://bucket/prefix".into(),
            auth: S3AuthConfig::AccessKey { key_id: "AKIA".into() },
            ..Default::default()
        };
        assert_eq!(
            register_source(&s, None).unwrap_err(),
            "access key secret is not set; re-enter it in Settings"
        );
    }

    /// Manual: COSTLYTICS_S3_TEST_URI=s3://bucket/prefix [COSTLYTICS_S3_TEST_PROFILE=p]
    /// [COSTLYTICS_S3_TEST_REGION=r] cargo test -p service -- --ignored real_s3
    #[test]
    #[ignore]
    fn real_s3_source_registers() {
        let uri = std::env::var("COSTLYTICS_S3_TEST_URI").expect("set COSTLYTICS_S3_TEST_URI");
        let s = DataSource {
            id: "s3".into(),
            name: "S3".into(),
            s3_uri: uri,
            aws_profile: std::env::var("COSTLYTICS_S3_TEST_PROFILE").ok(),
            aws_region: std::env::var("COSTLYTICS_S3_TEST_REGION").ok(),
            ..Default::default()
        };
        let r = register_source(&s, None).expect("S3 registration");
        println!("{r:?}");
        assert!(r.file_count > 0);
    }
}
