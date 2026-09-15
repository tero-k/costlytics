use thiserror::Error;

/// Grouping dimensions supported across all dashboards.
/// Maps to a fixed SQL identifier in normalized_cost — never accepts raw column names from HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    Service,
    Account,
    Region,
    AvailabilityZone,
    ChargeCategory,
    PricingCategory,
    Resource,
}

/// Error for unknown dimensions.
#[derive(Debug, Error)]
#[error("unknown dimension: {0}")]
pub struct UnknownDimension(pub String);

impl Dimension {
    /// Returns the canonical SQL column name for GROUP BY / SELECT.
    pub fn sql_column(&self) -> &'static str {
        match self {
            Dimension::Service => "service_name",
            Dimension::Account => "account_id",
            Dimension::Region => "region",
            Dimension::AvailabilityZone => "availability_zone",
            Dimension::ChargeCategory => "charge_category",
            Dimension::PricingCategory => "pricing_category",
            Dimension::Resource => "resource_id",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dimension_sql_column() {
        assert_eq!(Dimension::Service.sql_column(), "service_name");
        assert_eq!(Dimension::Account.sql_column(), "account_id");
        assert_eq!(Dimension::Region.sql_column(), "region");
        assert_eq!(Dimension::AvailabilityZone.sql_column(), "availability_zone");
        assert_eq!(Dimension::ChargeCategory.sql_column(), "charge_category");
        assert_eq!(Dimension::PricingCategory.sql_column(), "pricing_category");
        assert_eq!(Dimension::Resource.sql_column(), "resource_id");
    }
}
