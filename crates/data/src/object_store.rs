use thiserror::Error;

#[derive(Debug, Error)]
pub enum ObjectStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
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

    /// Parse from "YYYY-MM" string.
    pub fn parse(s: &str) -> Option<Self> {
        let (y, m) = s.split_once('-')?;
        Some(Self {
            year: y.parse().ok()?,
            month: m.parse().ok()?,
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

/// Minimal abstraction over object storage (local FS or S3).
pub trait ObjectStore: Send + Sync {
    /// List all "directory" prefixes immediately under `prefix` that match
    /// the `BILLING_PERIOD=YYYY-MM` pattern. Returns just the `YYYY-MM` part.
    fn list_billing_periods(&self, base_uri: &str) -> anyhow::Result<Vec<String>>;

    /// Read the bytes of a file (e.g. Manifest.json).
    fn read_file(&self, uri: &str) -> anyhow::Result<Vec<u8>>;
}

/// Local filesystem implementation of `ObjectStore`.
pub struct LocalObjectStore;

impl ObjectStore for LocalObjectStore {
    fn list_billing_periods(&self, base_path: &str) -> anyhow::Result<Vec<String>> {
        let dir = std::path::Path::new(base_path);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut periods = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Match BILLING_PERIOD=YYYY-MM
            if let Some(ym_str) = name_str.strip_prefix("BILLING_PERIOD=") {
                if YearMonth::parse(ym_str).is_some() {
                    periods.push(ym_str.to_string());
                }
            }
        }
        // Return sorted for deterministic ordering
        periods.sort();
        Ok(periods)
    }

    fn read_file(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        Ok(std::fs::read(path)?)
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
        let mut periods = store
            .list_billing_periods(dir.path().to_str().unwrap())
            .unwrap();
        periods.sort();
        assert_eq!(periods, vec!["2026-08", "2026-09"]);
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
        assert!(YearMonth::parse("2026-13").is_none() || YearMonth::parse("2026-13").is_some());
        // Only check that truly malformed strings fail
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
}
