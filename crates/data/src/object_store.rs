use thiserror::Error;

#[derive(Debug, Error)]
pub enum ObjectStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Query error: {0}")]
    Query(String),
}

/// A year+month pair used to identify billing period partitions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32, // 1-12
}

impl YearMonth {
    pub fn new(year: i32, month: u32) -> Self {
        Self { year, month }
    }

    /// Parse from "YYYY-MM" string. Returns `None` if month is not in 1..=12.
    pub fn parse(s: &str) -> Option<Self> {
        let (y, m) = s.split_once('-')?;
        let month: u32 = m.parse().ok()?;
        if !(1..=12).contains(&month) {
            return None;
        }
        Some(Self {
            year: y.parse().ok()?,
            month,
        })
    }

    /// Format as "YYYY-MM".
    pub fn to_ym_string(&self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    /// First day of this month as a NaiveDate.
    pub fn start_date(&self) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap()
    }

    /// First day of the NEXT month (exclusive end).
    pub fn end_date_exclusive(&self) -> chrono::NaiveDate {
        let (y, m) = if self.month == 12 {
            (self.year + 1, 1)
        } else {
            (self.year, self.month + 1)
        };
        chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap()
    }
}

impl std::fmt::Display for YearMonth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

/// One `BILLING_PERIOD=YYYY-MM` directory found under a source's base URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingPeriodDir {
    pub period: YearMonth,
    /// Full URI/path of the directory, spelled as it exists in storage
    /// (exports may write `billing_period=` in lower case).
    pub dir_uri: String,
}

/// Parses a directory name of the form `BILLING_PERIOD=YYYY-MM`, matching the
/// `BILLING_PERIOD=` prefix case-insensitively.
fn parse_billing_period_dir_name(name: &str) -> Option<YearMonth> {
    const PREFIX: &str = "BILLING_PERIOD=";
    let prefix = name.get(..PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    YearMonth::parse(&name[PREFIX.len()..])
}

/// Minimal abstraction over object storage (local FS or S3).
pub trait ObjectStore: Send + Sync {
    /// List the `BILLING_PERIOD=YYYY-MM` directories (prefix matched
    /// case-insensitively) immediately under `base_uri`, sorted by period and
    /// then path. A period may appear more than once if its directory exists
    /// in several spellings.
    fn list_billing_periods(&self, base_uri: &str) -> Result<Vec<BillingPeriodDir>, ObjectStoreError>;

    /// Read the bytes of a file (e.g. Manifest.json).
    fn read_file(&self, uri: &str) -> Result<Vec<u8>, ObjectStoreError>;

    /// List files directly under `dir_uri` whose name ends in `.{extension}`,
    /// as full URIs/paths, sorted. A missing directory yields an empty list.
    fn list_files(&self, dir_uri: &str, extension: &str) -> Result<Vec<String>, ObjectStoreError>;
}

/// Local filesystem implementation of `ObjectStore`.
pub struct LocalObjectStore;

impl ObjectStore for LocalObjectStore {
    fn list_billing_periods(&self, base_path: &str) -> Result<Vec<BillingPeriodDir>, ObjectStoreError> {
        let dir = std::path::Path::new(base_path);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let base = base_path.trim_end_matches('/').trim_end_matches('\\');
        let mut periods = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(period) = parse_billing_period_dir_name(&name) {
                periods.push(BillingPeriodDir { period, dir_uri: format!("{base}/{name}") });
            }
        }
        // Return sorted for deterministic ordering
        periods.sort_by(|a, b| (&a.period, &a.dir_uri).cmp(&(&b.period, &b.dir_uri)));
        Ok(periods)
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, ObjectStoreError> {
        Ok(std::fs::read(path)?)
    }

    fn list_files(&self, dir_uri: &str, extension: &str) -> Result<Vec<String>, ObjectStoreError> {
        let dir = std::path::Path::new(dir_uri);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let suffix = format!(".{extension}");
        let base = dir_uri.trim_end_matches('/').trim_end_matches('\\');
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(&suffix) {
                files.push(format!("{base}/{name}"));
            }
        }
        files.sort();
        Ok(files)
    }
}

/// `ObjectStore` backed by DuckDB's own filesystem layer (`glob()` and
/// `read_blob()`), so S3 listing/reading uses exactly the same httpfs
/// extension and `s3_secret` as the queries themselves — no AWS SDK. For an
/// `s3://` source, call `duckdb_pool::init_connection` on the pool first.
pub struct DuckDbObjectStore {
    pool: crate::duckdb_pool::DbPool,
}

impl DuckDbObjectStore {
    pub fn new(pool: crate::duckdb_pool::DbPool) -> Self {
        Self { pool }
    }

    fn glob(&self, pattern: &str) -> Result<Vec<String>, ObjectStoreError> {
        let conn = self.pool.get().map_err(|e| ObjectStoreError::Query(e.to_string()))?;
        let sql = format!("SELECT file FROM glob('{}') ORDER BY file", pattern.replace('\'', "''"));
        let mut stmt = conn.prepare(&sql).map_err(|e| ObjectStoreError::Query(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| ObjectStoreError::Query(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| ObjectStoreError::Query(e.to_string()))
    }
}

impl ObjectStore for DuckDbObjectStore {
    fn list_billing_periods(&self, base_uri: &str) -> Result<Vec<BillingPeriodDir>, ObjectStoreError> {
        // `glob()` patterns are case-sensitive, so list every file one level
        // down and match the directory name ourselves.
        let base = base_uri.trim_end_matches('/');
        let mut periods: Vec<BillingPeriodDir> = self
            .glob(&format!("{base}/*/*"))?
            .iter()
            .filter_map(|file| {
                let name = file.rsplit(['/', '\\']).nth(1)?;
                let period = parse_billing_period_dir_name(name)?;
                Some(BillingPeriodDir { period, dir_uri: format!("{base}/{name}") })
            })
            .collect();
        periods.sort_by(|a, b| (&a.period, &a.dir_uri).cmp(&(&b.period, &b.dir_uri)));
        periods.dedup();
        Ok(periods)
    }

    fn read_file(&self, uri: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let conn = self.pool.get().map_err(|e| ObjectStoreError::Query(e.to_string()))?;
        let sql = format!("SELECT content FROM read_blob('{}')", uri.replace('\'', "''"));
        conn.query_row(&sql, [], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|e| ObjectStoreError::Query(e.to_string()))
    }

    fn list_files(&self, dir_uri: &str, extension: &str) -> Result<Vec<String>, ObjectStoreError> {
        self.glob(&format!("{}/*.{}", dir_uri.trim_end_matches('/'), extension))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use std::fs;
    use tempfile::TempDir;

    fn setup_temp_store() -> TempDir {
        let dir = TempDir::new().unwrap();
        // Create valid billing period directory
        fs::create_dir(dir.path().join("BILLING_PERIOD=2026-08")).unwrap();
        fs::create_dir(dir.path().join("BILLING_PERIOD=2026-09")).unwrap();
        // Non-matching directories should be ignored
        fs::create_dir(dir.path().join("other-dir")).unwrap();
        fs::create_dir(dir.path().join("BILLING_PERIOD=not-a-date")).unwrap();
        dir
    }

    #[test]
    fn local_store_lists_billing_periods() {
        let dir = setup_temp_store();
        let store = LocalObjectStore;
        let base = dir.path().to_str().unwrap();
        let periods = store.list_billing_periods(base).unwrap();
        assert_eq!(
            periods,
            vec![
                BillingPeriodDir {
                    period: YearMonth::new(2026, 8),
                    dir_uri: format!("{base}/BILLING_PERIOD=2026-08"),
                },
                BillingPeriodDir {
                    period: YearMonth::new(2026, 9),
                    dir_uri: format!("{base}/BILLING_PERIOD=2026-09"),
                },
            ]
        );
    }

    #[test]
    fn billing_period_prefix_is_case_insensitive() {
        assert_eq!(parse_billing_period_dir_name("billing_period=2026-09"), Some(YearMonth::new(2026, 9)));
        assert_eq!(parse_billing_period_dir_name("Billing_Period=2025-04"), Some(YearMonth::new(2025, 4)));
        assert_eq!(parse_billing_period_dir_name("BILLING_PERIOD=2026-05"), Some(YearMonth::new(2026, 5)));
        assert_eq!(parse_billing_period_dir_name("billing_period=bogus"), None);
        assert_eq!(parse_billing_period_dir_name("period=2026-09"), None);
        assert_eq!(parse_billing_period_dir_name("é"), None);
    }

    #[test]
    fn local_store_lists_lowercase_dirs_with_their_real_spelling() {
        let dir = TempDir::new().unwrap();
        // Mixed spellings of *different* months: two spellings of the same
        // month can coexist on S3 but not on a case-insensitive local disk.
        fs::create_dir(dir.path().join("billing_period=2026-09")).unwrap();
        fs::create_dir(dir.path().join("BILLING_PERIOD=2026-05")).unwrap();
        let base = dir.path().to_str().unwrap();

        let dirs: Vec<(String, String)> = LocalObjectStore
            .list_billing_periods(base)
            .unwrap()
            .into_iter()
            .map(|d| (d.period.to_ym_string(), d.dir_uri))
            .collect();

        assert_eq!(
            dirs,
            vec![
                ("2026-05".into(), format!("{base}/BILLING_PERIOD=2026-05")),
                ("2026-09".into(), format!("{base}/billing_period=2026-09")),
            ]
        );
    }

    #[test]
    fn local_store_reads_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.json");
        fs::write(&file_path, b"hello world").unwrap();

        let store = LocalObjectStore;
        let bytes = store
            .read_file(file_path.to_str().unwrap())
            .unwrap();
        assert_eq!(bytes, b"hello world");
    }

    #[test]
    fn local_store_empty_dir() {
        let dir = TempDir::new().unwrap();
        let store = LocalObjectStore;
        let periods = store
            .list_billing_periods(dir.path().to_str().unwrap())
            .unwrap();
        assert!(periods.is_empty());
    }

    #[test]
    fn local_store_nonexistent_dir() {
        let store = LocalObjectStore;
        let periods = store
            .list_billing_periods("/nonexistent/path/that/does/not/exist")
            .unwrap();
        assert!(periods.is_empty());
    }

    #[test]
    fn year_month_parse_valid() {
        let ym = YearMonth::parse("2026-08").unwrap();
        assert_eq!(ym.year, 2026);
        assert_eq!(ym.month, 8);
    }

    #[test]
    fn year_month_parse_invalid() {
        assert!(YearMonth::parse("2026-13").is_none());
        assert!(YearMonth::parse("2026-00").is_none());
        assert!(YearMonth::parse("not-a-date").is_none());
        assert!(YearMonth::parse("2026").is_none());
        assert!(YearMonth::parse("").is_none());
    }

    #[test]
    fn year_month_display() {
        let ym = YearMonth::new(2026, 8);
        assert_eq!(ym.to_string(), "2026-08");
        assert_eq!(ym.to_ym_string(), "2026-08");
    }

    #[test]
    fn year_month_start_date() {
        let ym = YearMonth::new(2026, 8);
        let start = ym.start_date();
        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 8);
        assert_eq!(start.day(), 1);
    }

    #[test]
    fn year_month_end_date_exclusive_december() {
        let ym = YearMonth::new(2026, 12);
        let end = ym.end_date_exclusive();
        assert_eq!(end.year(), 2027);
        assert_eq!(end.month(), 1);
        assert_eq!(end.day(), 1);
    }

    #[test]
    fn local_store_lists_files_by_extension() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("b.parquet"), b"x").unwrap();
        fs::write(dir.path().join("a.parquet"), b"x").unwrap();
        fs::write(dir.path().join("Manifest.json"), b"{}").unwrap();
        let base = dir.path().to_str().unwrap();
        let files = LocalObjectStore.list_files(base, "parquet").unwrap();
        assert_eq!(files, vec![format!("{base}/a.parquet"), format!("{base}/b.parquet")]);
    }

    #[test]
    fn local_store_list_files_missing_dir_is_empty() {
        assert!(LocalObjectStore.list_files("/nonexistent/dir", "parquet").unwrap().is_empty());
    }

    #[test]
    fn duckdb_store_lists_periods_reads_and_lists_files() {
        let dir = TempDir::new().unwrap();
        let pd = dir.path().join("BILLING_PERIOD=2026-08");
        fs::create_dir(&pd).unwrap();
        fs::write(pd.join("Manifest.json"), b"{\"dataFiles\":[]}").unwrap();
        fs::write(pd.join("part-1.parquet"), b"x").unwrap();
        fs::create_dir(dir.path().join("BILLING_PERIOD=bogus")).unwrap();
        fs::write(dir.path().join("BILLING_PERIOD=bogus").join("f.parquet"), b"x").unwrap();

        let base = dir.path().to_str().unwrap().replace('\\', "/");
        let store = DuckDbObjectStore::new(crate::duckdb_pool::build_pool().unwrap());

        assert_eq!(
            store.list_billing_periods(&base).unwrap(),
            vec![BillingPeriodDir {
                period: YearMonth::new(2026, 8),
                dir_uri: format!("{base}/BILLING_PERIOD=2026-08"),
            }]
        );
        assert_eq!(
            store.read_file(&format!("{base}/BILLING_PERIOD=2026-08/Manifest.json")).unwrap(),
            b"{\"dataFiles\":[]}"
        );
        let files = store.list_files(&format!("{base}/BILLING_PERIOD=2026-08"), "parquet").unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("part-1.parquet"));
        assert!(store.read_file(&format!("{base}/missing.json")).is_err());
    }

    /// The S3 path: exports that write `billing_period=` in lower case.
    #[test]
    fn duckdb_store_lists_lowercase_billing_period_dirs() {
        let dir = TempDir::new().unwrap();
        for name in ["billing_period=2026-08", "billing_period=2026-09"] {
            let pd = dir.path().join(name);
            fs::create_dir(&pd).unwrap();
            fs::write(pd.join("part-1.parquet"), b"x").unwrap();
        }
        let base = dir.path().to_str().unwrap().replace('\\', "/");
        let store = DuckDbObjectStore::new(crate::duckdb_pool::build_pool().unwrap());

        assert_eq!(
            store.list_billing_periods(&base).unwrap(),
            vec![
                BillingPeriodDir {
                    period: YearMonth::new(2026, 8),
                    dir_uri: format!("{base}/billing_period=2026-08"),
                },
                BillingPeriodDir {
                    period: YearMonth::new(2026, 9),
                    dir_uri: format!("{base}/billing_period=2026-09"),
                },
            ]
        );
    }
}
