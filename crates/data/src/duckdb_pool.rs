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

/// Initialize a DuckDB connection for a given data source.
///
/// - For local (non-S3) sources: no extensions are loaded (fully offline).
/// - For S3 sources: loads `httpfs` + `aws`, then creates a credential-chain secret.
///
/// `aws_region` and `aws_profile` are optional overrides for the secret; both default
/// to the credential-chain discovery behaviour when omitted.
pub fn init_connection(
    conn: &Connection,
    is_s3: bool,
    aws_region: Option<&str>,
    aws_profile: Option<&str>,
) -> Result<(), duckdb::Error> {
    if is_s3 {
        conn.execute_batch("INSTALL httpfs; LOAD httpfs; INSTALL aws; LOAD aws;")?;
        let secret_sql = build_secret_sql(aws_region, aws_profile);
        conn.execute_batch(&secret_sql)?;
    }
    Ok(())
}

/// Build the `CREATE OR REPLACE SECRET` SQL for a credential-chain S3 secret.
fn build_secret_sql(region: Option<&str>, profile: Option<&str>) -> String {
    let mut parts = vec![
        "CREATE OR REPLACE SECRET s3_secret (".to_string(),
        "    TYPE S3,".to_string(),
        "    PROVIDER credential_chain".to_string(),
    ];
    if let Some(r) = region {
        parts.push(format!("    , REGION '{}'", r));
    }
    if let Some(p) = profile {
        parts.push(format!("    , PROFILE '{}'", p));
    }
    parts.push(");".to_string());
    parts.join("\n")
}

/// Build an in-memory DuckDB pool with 2 connections.
///
/// For S3 sources, call `init_connection` on the first connection obtained from
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
    use duckdb::Connection;

    #[test]
    fn test_build_secret_sql_no_options() {
        let sql = build_secret_sql(None, None);
        assert!(sql.contains("TYPE S3"));
        assert!(sql.contains("PROVIDER credential_chain"));
        assert!(!sql.contains("REGION"));
        assert!(!sql.contains("PROFILE"));
    }

    #[test]
    fn test_build_secret_sql_with_region() {
        let sql = build_secret_sql(Some("eu-west-1"), None);
        assert!(sql.contains("REGION 'eu-west-1'"));
        assert!(!sql.contains("PROFILE"));
    }

    #[test]
    fn test_build_secret_sql_with_profile() {
        let sql = build_secret_sql(None, Some("my-profile"));
        assert!(!sql.contains("REGION"));
        assert!(sql.contains("PROFILE 'my-profile'"));
    }

    #[test]
    fn test_build_secret_sql_with_both() {
        let sql = build_secret_sql(Some("us-east-1"), Some("prod"));
        assert!(sql.contains("REGION 'us-east-1'"));
        assert!(sql.contains("PROFILE 'prod'"));
    }

    #[test]
    fn test_init_connection_local_no_error() {
        // Local (non-S3) init must not fail — no extensions loaded.
        let conn = Connection::open_in_memory().unwrap();
        init_connection(&conn, false, None, None).unwrap();
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
