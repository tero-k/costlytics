/// Canonical column names used by adapter SQL and queries to stay in sync.
pub const COL_BILLING_PERIOD_START: &str = "billing_period_start";
pub const COL_BILLING_PERIOD_END: &str = "billing_period_end";
pub const COL_USAGE_START: &str = "usage_start";
pub const COL_USAGE_END: &str = "usage_end";
pub const COL_BILLING_ACCOUNT_ID: &str = "billing_account_id";
pub const COL_BILLING_ACCOUNT_NAME: &str = "billing_account_name";
pub const COL_ACCOUNT_ID: &str = "account_id";
pub const COL_ACCOUNT_NAME: &str = "account_name";
pub const COL_PROVIDER: &str = "provider";
pub const COL_PUBLISHER: &str = "publisher";
pub const COL_SERVICE_NAME: &str = "service_name";
pub const COL_SERVICE_CODE: &str = "service_code";
pub const COL_SERVICE_CATEGORY: &str = "service_category";
pub const COL_SERVICE_SUBCATEGORY: &str = "service_subcategory";
pub const COL_REGION: &str = "region";
pub const COL_AVAILABILITY_ZONE: &str = "availability_zone";
pub const COL_RESOURCE_ID: &str = "resource_id";
pub const COL_RESOURCE_NAME: &str = "resource_name";
pub const COL_RESOURCE_TYPE: &str = "resource_type";
pub const COL_CHARGE_CATEGORY: &str = "charge_category";
pub const COL_CHARGE_CLASS: &str = "charge_class";
pub const COL_CHARGE_FREQUENCY: &str = "charge_frequency";
pub const COL_CHARGE_DESCRIPTION: &str = "charge_description";
pub const COL_PRICING_CATEGORY: &str = "pricing_category";
pub const COL_USAGE_QUANTITY: &str = "usage_quantity";
pub const COL_USAGE_UNIT: &str = "usage_unit";
pub const COL_BILLED_COST: &str = "billed_cost";
pub const COL_AMORTIZED_COST: &str = "amortized_cost";
pub const COL_LIST_COST: &str = "list_cost";
pub const COL_CONTRACTED_COST: &str = "contracted_cost";
pub const COL_CURRENCY: &str = "currency";
pub const COL_TAGS: &str = "tags";
pub const COL_COMMITMENT_ID: &str = "commitment_id";
pub const COL_COMMITMENT_TYPE: &str = "commitment_type";
pub const COL_COMMITMENT_STATUS: &str = "commitment_status";
pub const COL_SOURCE_FORMAT: &str = "source_format";
pub const COL_SOURCE_FILE: &str = "source_file";

/// Metric type for cost queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CostMetric {
    #[default]
    Amortized,
    Billed,
    List,
    Contracted,
}

impl CostMetric {
    /// Returns the canonical column name for this metric in normalized_cost.
    pub fn column_name(&self) -> &'static str {
        match self {
            CostMetric::Amortized => COL_AMORTIZED_COST,
            CostMetric::Billed => COL_BILLED_COST,
            CostMetric::List => COL_LIST_COST,
            CostMetric::Contracted => COL_CONTRACTED_COST,
        }
    }
}

/// Summary of cost query results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CostSummary {
    pub metric: CostMetric,
    pub currency: String,
    pub total: f64,
    pub row_count: u64,
    pub source_format: Option<String>,
    pub query_ms: u64,
    /// Inclusive start of the queried window (UTC), echoed from the request filter.
    pub start: chrono::DateTime<chrono::Utc>,
    /// Exclusive end of the queried window (UTC), echoed from the request filter.
    pub end: chrono::DateTime<chrono::Utc>,
}

/// One point in a cost-over-time series.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimeSeriesPoint {
    /// Start of this period (UTC), truncated to the requested granularity.
    pub period: chrono::DateTime<chrono::Utc>,
    /// Optional grouping-dimension value for this point (e.g. a service name),
    /// present only when `timeseries()` was called with `grouping: Some(_)`.
    pub group: Option<String>,
    pub total: f64,
    pub row_count: u64,
}

/// One row in a dimension breakdown (e.g. "cost by service").
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BreakdownRow {
    /// The dimension's value for this row (e.g. a service name, account id).
    /// `None` represents rows where the canonical column was NULL in the source data.
    pub key: Option<String>,
    pub total: f64,
    pub row_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_metric_column_name() {
        assert_eq!(CostMetric::Amortized.column_name(), COL_AMORTIZED_COST);
        assert_eq!(CostMetric::Billed.column_name(), COL_BILLED_COST);
        assert_eq!(CostMetric::List.column_name(), COL_LIST_COST);
        assert_eq!(CostMetric::Contracted.column_name(), COL_CONTRACTED_COST);
    }

    #[test]
    fn test_cost_metric_default() {
        assert_eq!(CostMetric::default(), CostMetric::Amortized);
    }
}
