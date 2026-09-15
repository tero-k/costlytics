use chrono::{DateTime, Utc};
use crate::cost::CostMetric;

/// Operator for tag predicates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagOperator {
    Eq,
    Ne,
    Exists,
    NotExists,
    In,
}

/// A single tag filter predicate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TagFilter {
    pub key: String,
    pub operator: TagOperator,
    /// Values for Eq / Ne / In; empty for Exists / NotExists.
    pub values: Vec<String>,
}

/// Global filter shared across all dashboard queries.
/// All date comparisons use exclusive end: `usage_start >= start AND usage_start < end`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CostFilter {
    /// Inclusive start (UTC).
    pub start: DateTime<Utc>,
    /// Exclusive end (UTC).
    pub end: DateTime<Utc>,
    pub granularity: TimeGranularity,
    pub metric: CostMetric,
    pub accounts: Vec<String>,
    pub services: Vec<String>,
    pub regions: Vec<String>,
    pub tags: Vec<TagFilter>,
    pub charge_categories: Vec<String>,
    pub resource_ids: Vec<String>,
}

impl CostFilter {
    /// Minimal constructor for the common case.
    pub fn date_range(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self {
            start,
            end,
            granularity: TimeGranularity::default(),
            metric: CostMetric::default(),
            accounts: vec![],
            services: vec![],
            regions: vec![],
            tags: vec![],
            charge_categories: vec![],
            resource_ids: vec![],
        }
    }
}

/// Time granularity for cost aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeGranularity {
    Day,
    #[default]
    Month,
    Year,
}

impl TimeGranularity {
    /// Returns the SQL date_trunc unit for this granularity.
    pub fn date_trunc_unit(&self) -> &'static str {
        match self {
            TimeGranularity::Day => "day",
            TimeGranularity::Month => "month",
            TimeGranularity::Year => "year",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_granularity_date_trunc_unit() {
        assert_eq!(TimeGranularity::Day.date_trunc_unit(), "day");
        assert_eq!(TimeGranularity::Month.date_trunc_unit(), "month");
        assert_eq!(TimeGranularity::Year.date_trunc_unit(), "year");
    }

    #[test]
    fn test_time_granularity_default() {
        assert_eq!(TimeGranularity::default(), TimeGranularity::Month);
    }

    #[test]
    fn test_cost_filter_date_range() {
        let start = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let filter = CostFilter::date_range(start, end);

        assert_eq!(filter.start, start);
        assert_eq!(filter.end, end);
        assert_eq!(filter.granularity, TimeGranularity::Month);
        assert_eq!(filter.metric, CostMetric::Amortized);
        assert!(filter.accounts.is_empty());
        assert!(filter.services.is_empty());
        assert!(filter.regions.is_empty());
        assert!(filter.tags.is_empty());
        assert!(filter.charge_categories.is_empty());
        assert!(filter.resource_ids.is_empty());
    }
}
