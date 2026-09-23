use duckdb::{Connection, DuckdbConnectionManager};
use thiserror::Error;

/// The connection pool type for DuckDB.
pub type DbPool = r2d2::Pool<DuckdbConnectionManager>;
/// A pooled DuckDB connection.
pub type PooledConn = r2d2::PooledConnection<DuckdbConnectionManager>;

/// Errors from pool construction or query execution.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("DuckDB error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("Connection pool error: {0}")]
    Pool(#[from] r2d2::Error),
}

/// Credentials DuckDB uses to read an `s3://` source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3Auth {
    /// AWS credential chain (env, `~/.aws` profiles/SSO, instance role).
    CredentialChain {
        profile: Option<String>,
        region: Option<String>,
    },
    /// Static access key id + secret.
    AccessKey {
        key_id: String,
        secret: String,
        region: Option<String>,
    },
}

/// Quote `s` as a DuckDB string literal (`'` doubled).
fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn non_blank(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Prepare `conn` (and, since every pooled connection shares one in-memory
/// database, the whole pool) to read S3: loads `httpfs` (plus `aws` for the
/// credential chain) and creates the `s3_secret`. The extensions are
/// downloaded on first use and cached under `~/.duckdb/extensions`.
pub fn init_connection(conn: &Connection, auth: &S3Auth) -> Result<(), duckdb::Error> {
    conn.execute_batch("INSTALL httpfs; LOAD httpfs;")?;
    if matches!(auth, S3Auth::CredentialChain { .. }) {
        conn.execute_batch("INSTALL aws; LOAD aws;")?;
    }
    conn.execute_batch(&build_secret_sql(auth))?;
    // Best effort: cuts repeated HEAD requests when the same files are
    // queried again. Not fatal if a future DuckDB drops the setting.
    if let Err(e) = conn.execute_batch("SET enable_http_metadata_cache = true;") {
        tracing::debug!(error = %e, "enable_http_metadata_cache not supported");
    }
    Ok(())
}

/// Build the `CREATE OR REPLACE SECRET` statement for `auth`. Every value is
/// escaped with [`sql_str`].
pub fn build_secret_sql(auth: &S3Auth) -> String {
    let mut parts = vec!["TYPE S3".to_string()];
    let region = match auth {
        S3Auth::CredentialChain { profile, region } => {
            parts.push("PROVIDER credential_chain".to_string());
            if let Some(p) = non_blank(profile) {
                parts.push(format!("PROFILE {}", sql_str(p)));
            }
            region
        }
        S3Auth::AccessKey { key_id, secret, region } => {
            parts.push(format!("KEY_ID {}", sql_str(key_id)));
            parts.push(format!("SECRET {}", sql_str(secret)));
            region
        }
    };
    if let Some(r) = non_blank(region) {
        parts.push(format!("REGION {}", sql_str(r)));
    }
    format!("CREATE OR REPLACE SECRET s3_secret ({});", parts.join(", "))
}

/// Build an in-memory DuckDB pool with 2 connections.
///
/// For S3 sources, call `init_connection(&conn, &auth)` on the first connection obtained from
/// the pool immediately after building — because all pool connections share the
/// same underlying in-memory DuckDB instance, the initialisation applies to all.
pub fn build_pool() -> Result<DbPool, DbError> {
    let manager = DuckdbConnectionManager::memory()?;
    let pool = r2d2::Pool::builder()
        .max_size(2)
        .build(manager)?;
    Ok(pool)
}

/// Build a `read_parquet([...])` SQL fragment from a list of file paths.
///
/// The paths are embedded as string literals — this is safe because file paths
/// are not user-supplied input in the Costlytics context.
///
/// # Example
/// ```
/// use data::duckdb_pool::build_parquet_list;
/// let sql = build_parquet_list(&["a.parquet".to_string(), "b.parquet".to_string()]);
/// assert_eq!(sql, "['a.parquet', 'b.parquet']");
/// ```
pub fn build_parquet_list(files: &[String]) -> String {
    let items: Vec<String> = files
        .iter()
        .map(|f| format!("'{}'", f.replace('\'', "''")))
        .collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(profile: Option<&str>, region: Option<&str>) -> S3Auth {
        S3Auth::CredentialChain {
            profile: profile.map(str::to_string),
            region: region.map(str::to_string),
        }
    }

    #[test]
    fn secret_sql_credential_chain_minimal() {
        let sql = build_secret_sql(&chain(None, None));
        assert!(sql.starts_with("CREATE OR REPLACE SECRET s3_secret ("));
        assert!(sql.contains("TYPE S3"));
        assert!(sql.contains("PROVIDER credential_chain"));
        assert!(!sql.contains("REGION"));
        assert!(!sql.contains("PROFILE"));
    }

    #[test]
    fn secret_sql_credential_chain_with_profile_and_region() {
        let sql = build_secret_sql(&chain(Some("prod"), Some("us-east-1")));
        assert!(sql.contains("PROFILE 'prod'"));
        assert!(sql.contains("REGION 'us-east-1'"));
    }

    #[test]
    fn secret_sql_blank_profile_is_omitted() {
        let sql = build_secret_sql(&chain(Some(""), Some("")));
        assert!(!sql.contains("PROFILE"));
        assert!(!sql.contains("REGION"));
    }

    #[test]
    fn secret_sql_access_key() {
        let sql = build_secret_sql(&S3Auth::AccessKey {
            key_id: "AKIAEXAMPLE".into(),
            secret: "s3cr3t".into(),
            region: Some("eu-north-1".into()),
        });
        assert!(sql.contains("KEY_ID 'AKIAEXAMPLE'"));
        assert!(sql.contains("SECRET 's3cr3t'"));
        assert!(sql.contains("REGION 'eu-north-1'"));
        assert!(!sql.contains("credential_chain"));
    }

    #[test]
    fn secret_sql_escapes_single_quotes() {
        let sql = build_secret_sql(&S3Auth::AccessKey {
            key_id: "a'b".into(),
            secret: "x'); DROP TABLE t; --".into(),
            region: None,
        });
        assert!(sql.contains("KEY_ID 'a''b'"));
        assert!(sql.contains("SECRET 'x''); DROP TABLE t; --'"));
    }

    #[test]
    fn test_build_parquet_list_empty() {
        let result = build_parquet_list(&[]);
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_build_parquet_list_single() {
        let result = build_parquet_list(&["a.parquet".to_string()]);
        assert_eq!(result, "['a.parquet']");
    }

    #[test]
    fn test_build_parquet_list_multiple() {
        let result = build_parquet_list(&[
            "s3://bucket/a.parquet".to_string(),
            "s3://bucket/b.parquet".to_string(),
        ]);
        assert_eq!(result, "['s3://bucket/a.parquet', 's3://bucket/b.parquet']");
    }

    #[test]
    fn test_build_pool_constructs_successfully() {
        let pool = build_pool().unwrap();
        // Should be able to get a connection and run a simple query
        let conn = pool.get().unwrap();
        conn.execute_batch("SELECT 1").unwrap();
    }
}
