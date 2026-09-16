//! Builds a dynamic SQL predicate fragment (plus bound parameters) from a
//! `CostFilter`'s non-date fields (accounts/services/regions/tags/
//! charge_categories/resource_ids). See `tasks-predicates.md`'s "Design:
//! predicate semantics" section for the exact, binding semantics implemented
//! here.

use domain::cost::{
    COL_ACCOUNT_ID, COL_CHARGE_CATEGORY, COL_REGION, COL_RESOURCE_ID, COL_SERVICE_NAME, COL_TAGS,
};
use domain::filters::{CostFilter, TagFilter, TagOperator};

/// A dynamically-built SQL predicate fragment plus its bound parameter values,
/// derived from a `CostFilter`'s non-date fields (accounts/services/regions/
/// tags/charge_categories/resource_ids).
///
/// `sql` is either empty (no predicates) or starts with " AND " so it can be
/// appended directly after an existing WHERE clause's date-range conditions.
/// `params` are in the exact order their `?` placeholders appear in `sql`.
///
/// `params` is `Vec<String>` rather than `Vec<Box<dyn duckdb::ToSql>>` (or
/// similar): every predicate value in this domain (account IDs, service
/// names, tag keys/values, ...) is already a `String` in `CostFilter`, so a
/// homogeneous `Vec<String>` is both the simplest possible shape and — per
/// the empirical test below — composes directly with
/// `duckdb::params_from_iter`, including alongside the existing fixed-arity
/// date-range/limit params (just `.chain()` two iterators of `&dyn ToSql`,
/// or collect everything into one `Vec<&dyn ToSql>` before calling
/// `params_from_iter`). No need for a boxed/dynamic-typed param type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterPredicate {
    pub sql: String,
    pub params: Vec<String>,
}

/// Build a `FilterPredicate` from a `CostFilter`'s non-date fields. See the
/// module-level "Design: predicate semantics" doc for exact clause shapes.
pub fn build_predicate(filter: &CostFilter) -> FilterPredicate {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();

    push_list_clause(&mut clauses, &mut params, COL_ACCOUNT_ID, &filter.accounts);
    push_list_clause(
        &mut clauses,
        &mut params,
        COL_SERVICE_NAME,
        &filter.services,
    );
    push_list_clause(&mut clauses, &mut params, COL_REGION, &filter.regions);
    push_list_clause(
        &mut clauses,
        &mut params,
        COL_CHARGE_CATEGORY,
        &filter.charge_categories,
    );
    push_list_clause(
        &mut clauses,
        &mut params,
        COL_RESOURCE_ID,
        &filter.resource_ids,
    );

    for tag in &filter.tags {
        push_tag_clause(&mut clauses, &mut params, tag);
    }

    if clauses.is_empty() {
        FilterPredicate {
            sql: String::new(),
            params: Vec::new(),
        }
    } else {
        FilterPredicate {
            sql: format!(" AND {}", clauses.join(" AND ")),
            params,
        }
    }
}

/// Append an `{column} IN (?, ?, ...)` clause (and its bound values) for a
/// simple list field, or do nothing if `values` is empty ("no filter on this
/// dimension" — never emit an always-false `IN ()`).
fn push_list_clause(
    clauses: &mut Vec<String>,
    params: &mut Vec<String>,
    column: &str,
    values: &[String],
) {
    if values.is_empty() {
        return;
    }
    let placeholders = std::iter::repeat_n("?", values.len())
        .collect::<Vec<_>>()
        .join(", ");
    clauses.push(format!("{column} IN ({placeholders})"));
    params.extend(values.iter().cloned());
}

/// Append the clause (and bound params) for a single `TagFilter`, per the
/// tag-operator semantics in "Design: predicate semantics".
fn push_tag_clause(clauses: &mut Vec<String>, params: &mut Vec<String>, tag: &TagFilter) {
    match tag.operator {
        TagOperator::Eq => {
            // Spec looseness: if `values` has more than one entry, only
            // `values[0]` is used (documented, not an error). A `Vec` with
            // no values at all silently produces no clause for this tag
            // filter (nothing to compare equal to).
            if let Some(value) = tag.values.first() {
                clauses.push(format!("{COL_TAGS}[?] = ?"));
                params.push(tag.key.clone());
                params.push(value.clone());
            }
        }
        TagOperator::Ne => {
            // `IS DISTINCT FROM`, not `!=`: a row missing this tag key
            // entirely has `tags[key]` evaluate to SQL NULL, and
            // `NULL != 'x'` is NULL (row excluded) — wrong, since "no such
            // tag" is legitimately "not equal to X". `IS DISTINCT FROM`
            // treats a missing tag as correctly not-equal.
            if let Some(value) = tag.values.first() {
                clauses.push(format!("{COL_TAGS}[?] IS DISTINCT FROM ?"));
                params.push(tag.key.clone());
                params.push(value.clone());
            }
        }
        TagOperator::Exists => {
            clauses.push(format!("{COL_TAGS}[?] IS NOT NULL"));
            params.push(tag.key.clone());
        }
        TagOperator::NotExists => {
            clauses.push(format!("{COL_TAGS}[?] IS NULL"));
            params.push(tag.key.clone());
        }
        TagOperator::In => {
            // Empty `values` for `In` is treated as "not specified" (same
            // "empty list = no filter" convention as the simple list
            // fields) rather than an always-false predicate — silently
            // skip this tag filter entirely.
            if tag.values.is_empty() {
                return;
            }
            let placeholders = std::iter::repeat_n("?", tag.values.len())
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("{COL_TAGS}[?] IN ({placeholders})"));
            params.push(tag.key.clone());
            params.extend(tag.values.iter().cloned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn base_filter() -> CostFilter {
        CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        )
    }

    #[test]
    fn empty_filter_produces_no_predicate() {
        let filter = base_filter();
        let p = build_predicate(&filter);
        assert!(p.sql.is_empty());
        assert!(p.params.is_empty());
    }

    #[test]
    fn single_account_filter() {
        let mut filter = base_filter();
        filter.accounts = vec!["acct-1".to_string()];
        let p = build_predicate(&filter);
        assert!(p.sql.contains("account_id IN (?)"), "sql: {}", p.sql);
        assert_eq!(p.params, vec!["acct-1".to_string()]);
    }

    #[test]
    fn multiple_services_filter() {
        let mut filter = base_filter();
        filter.services = vec!["EC2".to_string(), "S3".to_string(), "RDS".to_string()];
        let p = build_predicate(&filter);
        assert!(
            p.sql.contains("service_name IN (?, ?, ?)"),
            "sql: {}",
            p.sql
        );
        assert_eq!(
            p.params,
            vec!["EC2".to_string(), "S3".to_string(), "RDS".to_string()]
        );
    }

    #[test]
    fn combined_list_filters_are_anded() {
        let mut filter = base_filter();
        filter.accounts = vec!["acct-1".to_string()];
        filter.services = vec!["EC2".to_string()];
        filter.regions = vec!["us-east-1".to_string()];
        let p = build_predicate(&filter);
        assert!(p.sql.contains("account_id IN (?)"), "sql: {}", p.sql);
        assert!(p.sql.contains("service_name IN (?)"), "sql: {}", p.sql);
        assert!(p.sql.contains("region IN (?)"), "sql: {}", p.sql);
        // Clauses joined with AND, in field-declaration order.
        assert_eq!(
            p.sql,
            " AND account_id IN (?) AND service_name IN (?) AND region IN (?)"
        );
        assert_eq!(
            p.params,
            vec![
                "acct-1".to_string(),
                "EC2".to_string(),
                "us-east-1".to_string()
            ]
        );
    }

    #[test]
    fn tag_eq_predicate() {
        let mut filter = base_filter();
        filter.tags = vec![TagFilter {
            key: "Environment".to_string(),
            operator: TagOperator::Eq,
            values: vec!["production".to_string()],
        }];
        let p = build_predicate(&filter);
        assert!(p.sql.contains("tags[?] = ?"), "sql: {}", p.sql);
        assert_eq!(
            p.params,
            vec!["Environment".to_string(), "production".to_string()]
        );
    }

    #[test]
    fn tag_ne_predicate() {
        let mut filter = base_filter();
        filter.tags = vec![TagFilter {
            key: "Environment".to_string(),
            operator: TagOperator::Ne,
            values: vec!["production".to_string()],
        }];
        let p = build_predicate(&filter);
        assert!(
            p.sql.contains("tags[?] IS DISTINCT FROM ?"),
            "sql: {}",
            p.sql
        );
        assert_eq!(
            p.params,
            vec!["Environment".to_string(), "production".to_string()]
        );
    }

    #[test]
    fn tag_exists_predicate() {
        let mut filter = base_filter();
        filter.tags = vec![TagFilter {
            key: "Environment".to_string(),
            operator: TagOperator::Exists,
            values: vec![],
        }];
        let p = build_predicate(&filter);
        assert!(p.sql.contains("tags[?] IS NOT NULL"), "sql: {}", p.sql);
        assert_eq!(p.params, vec!["Environment".to_string()]);
    }

    #[test]
    fn tag_not_exists_predicate() {
        let mut filter = base_filter();
        filter.tags = vec![TagFilter {
            key: "Environment".to_string(),
            operator: TagOperator::NotExists,
            values: vec![],
        }];
        let p = build_predicate(&filter);
        assert!(p.sql.contains("tags[?] IS NULL"), "sql: {}", p.sql);
        assert_eq!(p.params, vec!["Environment".to_string()]);
    }

    #[test]
    fn tag_in_predicate() {
        let mut filter = base_filter();
        filter.tags = vec![TagFilter {
            key: "Team".to_string(),
            operator: TagOperator::In,
            values: vec![
                "platform".to_string(),
                "payments".to_string(),
                "core".to_string(),
            ],
        }];
        let p = build_predicate(&filter);
        assert!(p.sql.contains("tags[?] IN (?, ?, ?)"), "sql: {}", p.sql);
        assert_eq!(
            p.params,
            vec![
                "Team".to_string(),
                "platform".to_string(),
                "payments".to_string(),
                "core".to_string(),
            ]
        );
    }

    #[test]
    fn tag_in_predicate_empty_values_is_skipped() {
        let mut filter = base_filter();
        filter.tags = vec![TagFilter {
            key: "Team".to_string(),
            operator: TagOperator::In,
            values: vec![],
        }];
        let p = build_predicate(&filter);
        assert!(p.sql.is_empty(), "sql: {}", p.sql);
        assert!(p.params.is_empty());
    }

    #[test]
    fn multiple_tag_filters_are_anded() {
        let mut filter = base_filter();
        filter.tags = vec![
            TagFilter {
                key: "Environment".to_string(),
                operator: TagOperator::Eq,
                values: vec!["production".to_string()],
            },
            TagFilter {
                key: "Team".to_string(),
                operator: TagOperator::Eq,
                values: vec!["payments".to_string()],
            },
        ];
        let p = build_predicate(&filter);
        assert_eq!(p.sql, " AND tags[?] = ? AND tags[?] = ?", "sql: {}", p.sql);
        assert_eq!(
            p.params,
            vec![
                "Environment".to_string(),
                "production".to_string(),
                "Team".to_string(),
                "payments".to_string(),
            ]
        );
    }

    // -------------------------------------------------------------------
    // Empirical proof: `duckdb::params_from_iter` accepts a runtime-
    // determined number of bound parameters against a real in-memory
    // connection, and `FilterPredicate::params: Vec<String>` composes
    // cleanly with it (including alongside additional fixed params, as
    // Task 2/3 will need when combining this with the existing date-range
    // params).
    // -------------------------------------------------------------------
    #[test]
    fn dynamic_param_binding_works_via_params_from_iter() {
        let conn = duckdb::Connection::open_in_memory().unwrap();

        // Three dynamically-bound string parameters, count only known at
        // runtime (built from a Vec, not literal at the call site).
        let values: Vec<String> = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let placeholders = std::iter::repeat_n("?", values.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT ? IN ({placeholders})", placeholders = placeholders);

        // The needle plus the dynamic haystack, in one call, proving a
        // FilterPredicate::params Vec<String> composes fine with an extra
        // fixed-arity param placed before it.
        let mut all_params: Vec<String> = vec!["b".to_string()];
        all_params.extend(values.clone());

        let mut stmt = conn.prepare(&sql).unwrap();
        let found: bool = stmt
            .query_row(duckdb::params_from_iter(all_params.iter()), |row| {
                row.get(0)
            })
            .unwrap();
        assert!(found);

        // Also confirm a miss correctly returns false, not an error.
        let mut all_params_miss: Vec<String> = vec!["z".to_string()];
        all_params_miss.extend(values);
        let mut stmt2 = conn.prepare(&sql).unwrap();
        let found2: bool = stmt2
            .query_row(duckdb::params_from_iter(all_params_miss.iter()), |row| {
                row.get(0)
            })
            .unwrap();
        assert!(!found2);
    }

    /// Confirms `FilterPredicate`'s produced `sql`/`params` are directly
    /// usable to filter a real DuckDB table: build a predicate from a
    /// `CostFilter`, splice it after a date-range WHERE clause, and combine
    /// its dynamic params with the existing fixed date-range params via
    /// `params_from_iter` over `&dyn duckdb::ToSql` trait objects — proving
    /// the composition Task 2/3 needs.
    #[test]
    fn filter_predicate_composes_with_fixed_date_params_against_real_table() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (usage_start VARCHAR, account_id VARCHAR); \
             INSERT INTO t VALUES ('2026-08-10', 'acct-1'), ('2026-08-11', 'acct-2');",
        )
        .unwrap();

        let mut filter = base_filter();
        filter.accounts = vec!["acct-1".to_string()];
        let predicate = build_predicate(&filter);

        let sql = format!(
            "SELECT COUNT(*) FROM t WHERE usage_start >= ? {}",
            predicate.sql
        );

        // Fixed date param + the predicate's dynamic params, combined into
        // one Vec<&dyn ToSql> for a single params_from_iter call.
        let start_str = "2026-08-01".to_string();
        let mut combined: Vec<&dyn duckdb::ToSql> = vec![&start_str];
        for p in &predicate.params {
            combined.push(p);
        }

        let mut stmt = conn.prepare(&sql).unwrap();
        let count: i64 = stmt
            .query_row(duckdb::params_from_iter(combined.iter()), |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
