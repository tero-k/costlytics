//! Spend anomaly detection over one scope's daily cost series (pure, no I/O).
//!
//! A day is anomalous only when it is BOTH statistically unusual against a
//! robust baseline (weekday-adjusted median, MAD spread) AND materially
//! large (absolute, relative and optional monthly-impact floors), so a
//! $0.10 → $1 jump never fires however large the ratio. A series too
//! sparse for a baseline is compared against its largest recent day, and an
//! upward finding that repeats last month's same-day charge is dropped. See
//! `docs/superpowers/specs/2026-10-18-spend-anomaly-detection-design.md`.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Months, NaiveDate};
use serde::{Deserialize, Serialize};

/// Days with any cost row a series needs before it gets a statistical baseline.
pub const MIN_HISTORY_DAYS: usize = 14;
/// Projected monthly impact = daily delta × this.
pub const DAYS_PER_MONTH: f64 = 30.0;
/// Days before an evaluated day that detection may read beyond the lookback
/// window: `new_spend` remembers this many days, and the month echo looks
/// back one calendar month ± 1 day (at most 32 days). A check's query must
/// reach `max(lookback_days, MONTH_ECHO_REACH_DAYS)` days before its first day.
pub const MONTH_ECHO_REACH_DAYS: i64 = 35;
/// Scales MAD to a standard-deviation equivalent for normally distributed data.
const MAD_SCALE: f64 = 1.4826;
/// Spread floor as a share of expected, so a perfectly flat series (MAD = 0)
/// doesn't flag every cent of wobble.
const SPREAD_FLOOR_SHARE: f64 = 0.05;
/// Bounds on the weekday factor: ~4 samples per weekday is few.
const WEEKDAY_FACTOR_MIN: f64 = 0.2;
const WEEKDAY_FACTOR_MAX: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Service,
    Account,
}

impl ScopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScopeKind::Service => "service",
            ScopeKind::Account => "account",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyKind {
    Spike,
    Drop,
    /// Spend appeared where there was (almost) none in the lookback window.
    NewSpend,
}

impl AnomalyKind {
    pub fn direction(&self) -> Direction {
        match self {
            AnomalyKind::Spike | AnomalyKind::NewSpend => Direction::Up,
            AnomalyKind::Drop => Direction::Down,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

/// Resolved detection rules for one series (global defaults plus any override).
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub lookback_days: u32,
    pub mad_multiplier: f64,
    pub min_delta_usd: f64,
    pub min_delta_pct: f64,
    /// 0 = rule off.
    pub min_monthly_impact_usd: f64,
    pub include_drops: bool,
    pub severity_medium_usd: f64,
    pub severity_high_usd: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            lookback_days: 28,
            mad_multiplier: 3.0,
            min_delta_usd: 20.0,
            min_delta_pct: 0.25,
            min_monthly_impact_usd: 0.0,
            include_drops: false,
            severity_medium_usd: 500.0,
            severity_high_usd: 2000.0,
        }
    }
}

/// One anomalous day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub day: NaiveDate,
    pub expected: f64,
    pub actual: f64,
    /// `actual - expected`.
    pub delta: f64,
    pub kind: AnomalyKind,
    pub severity: Severity,
}

/// One row of the per-service / per-account daily series query.
#[derive(Debug, Clone, PartialEq)]
pub struct DailySeriesRow {
    pub scope_kind: ScopeKind,
    pub scope_key: String,
    pub day: NaiveDate,
    pub cost: f64,
}

/// Severity from projected monthly impact (`|delta| × 30`).
pub fn severity(delta: f64, rules: &Rules) -> Severity {
    let monthly = delta.abs() * DAYS_PER_MONTH;
    if monthly >= rules.severity_high_usd {
        Severity::High
    } else if monthly >= rules.severity_medium_usd {
        Severity::Medium
    } else {
        Severity::Low
    }
}

/// Evaluates every day in `from..=to`. Days missing from `series` count as $0.
/// The series should reach [`MONTH_ECHO_REACH_DAYS`] (or the lookback, if
/// longer) before `from`.
pub fn detect(
    series: &BTreeMap<NaiveDate, f64>,
    from: NaiveDate,
    to: NaiveDate,
    rules: &Rules,
) -> Vec<Finding> {
    from.iter_days()
        .take_while(|d| *d <= to)
        .filter_map(|d| evaluate_day(series, d, rules))
        .collect()
}

/// One day's finding, if any. An upward finding is dropped when it echoes
/// last month (see [`month_echo`]): a charge billed on the same day each
/// month is not news.
pub fn evaluate_day(
    series: &BTreeMap<NaiveDate, f64>,
    day: NaiveDate,
    rules: &Rules,
) -> Option<Finding> {
    candidate(series, day, rules).filter(|f| {
        f.kind.direction() == Direction::Down || !month_echo(series, f.day, f.actual, rules)
    })
}

/// Whether a day within one day of the same date last month (chrono clamps
/// the 31st to the month's end) cost at least `(1 − min_delta_pct) × actual`.
fn month_echo(
    series: &BTreeMap<NaiveDate, f64>,
    day: NaiveDate,
    actual: f64,
    rules: &Rules,
) -> bool {
    let Some(center) = day.checked_sub_months(Months::new(1)) else {
        return false;
    };
    let threshold = (1.0 - rules.min_delta_pct) * actual;
    (-1..=1)
        .map(|i| center + Duration::days(i))
        .filter_map(|d| series.get(&d))
        // A day without spend never echoes, even when `min_delta_pct` ≥ 1 makes the threshold ≤ 0.
        .any(|&c| c > 0.0 && c >= threshold)
}

fn candidate(series: &BTreeMap<NaiveDate, f64>, day: NaiveDate, rules: &Rules) -> Option<Finding> {
    let cost = |d: NaiveDate| series.get(&d).copied().unwrap_or(0.0);
    let window: Vec<NaiveDate> = (1..=i64::from(rules.lookback_days))
        .map(|i| day - Duration::days(i))
        .collect();
    if window.is_empty() {
        return None;
    }
    let actual = cost(day);
    let values: Vec<f64> = window.iter().map(|d| cost(*d)).collect();
    let history = window.iter().filter(|d| series.contains_key(d)).count();
    let base = median(&values);

    if history < MIN_HISTORY_DAYS || base <= 0.0 {
        // Both rules see the same days: spend that only new_spend remembers
        // (beyond the window) would otherwise block it without giving the
        // sporadic rule anything to compare against.
        let memory = i64::from(rules.lookback_days).max(MONTH_ECHO_REACH_DAYS);
        let prior: Vec<f64> = (1..=memory).map(|i| cost(day - Duration::days(i))).collect();
        let prior_total = prior.iter().sum();
        let largest = prior.iter().copied().fold(0.0, f64::max);
        return new_spend(day, actual, prior_total, rules)
            .or_else(|| sporadic_spike(day, actual, largest, rules));
    }

    // Per-weekday factor relative to the overall median, indexed Mon=0..Sun=6.
    let mut factors = [1.0_f64; 7];
    for (i, factor) in factors.iter_mut().enumerate() {
        let same: Vec<f64> = window
            .iter()
            .filter(|d| d.weekday().num_days_from_monday() as usize == i)
            .map(|d| cost(*d))
            .collect();
        if !same.is_empty() {
            *factor = (median(&same) / base).clamp(WEEKDAY_FACTOR_MIN, WEEKDAY_FACTOR_MAX);
        }
    }
    let factor_of = |d: NaiveDate| factors[d.weekday().num_days_from_monday() as usize];
    let residuals: Vec<f64> = window
        .iter()
        .map(|d| (cost(*d) - base * factor_of(*d)).abs())
        .collect();

    let expected = base * factor_of(day);
    let spread = (MAD_SCALE * median(&residuals)).max(SPREAD_FLOOR_SHARE * expected);
    let delta = actual - expected;
    let magnitude = delta.abs();
    let material = magnitude > 0.0
        && magnitude >= rules.mad_multiplier * spread
        && magnitude >= rules.min_delta_usd
        && magnitude / expected >= rules.min_delta_pct
        && magnitude * DAYS_PER_MONTH >= rules.min_monthly_impact_usd;
    if !material || (delta < 0.0 && !rules.include_drops) {
        return None;
    }
    let kind = if delta > 0.0 {
        AnomalyKind::Spike
    } else {
        AnomalyKind::Drop
    };
    Some(Finding {
        day,
        expected,
        actual,
        delta,
        kind,
        severity: severity(delta, rules),
    })
}

/// No usable baseline: fires only for spend that appears out of (almost)
/// nothing in the last `max(lookback_days, MONTH_ECHO_REACH_DAYS)` days.
fn new_spend(day: NaiveDate, actual: f64, prior_total: f64, rules: &Rules) -> Option<Finding> {
    let fires = actual > 0.0
        && actual >= rules.min_delta_usd
        && actual * DAYS_PER_MONTH >= rules.min_monthly_impact_usd
        && prior_total < rules.min_delta_usd.max(0.01);
    fires.then(|| Finding {
        day,
        expected: 0.0,
        actual,
        delta: actual,
        kind: AnomalyKind::NewSpend,
        severity: severity(actual, rules),
    })
}

/// No usable baseline but some recent spend: fires when the day clears the
/// largest day of the last `max(lookback_days, MONTH_ECHO_REACH_DAYS)` days
/// by the absolute, relative and monthly-impact floors.
fn sporadic_spike(day: NaiveDate, actual: f64, largest: f64, rules: &Rules) -> Option<Finding> {
    let delta = actual - largest;
    let fires = largest > 0.0
        && delta >= rules.min_delta_usd
        && delta / largest >= rules.min_delta_pct
        && delta * DAYS_PER_MONTH >= rules.min_monthly_impact_usd;
    fires.then(|| Finding {
        day,
        expected: largest,
        actual,
        delta,
        kind: AnomalyKind::Spike,
        severity: severity(delta, rules),
    })
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// `days` consecutive days ending the day before `check`, all at `value`.
    fn flat(check: NaiveDate, days: i64, value: f64) -> BTreeMap<NaiveDate, f64> {
        (1..=days)
            .map(|i| (check - Duration::days(i), value))
            .collect()
    }

    const CHECK: fn() -> NaiveDate = || d(2026, 9, 30);

    #[test]
    fn spike_on_flat_series_fires_high() {
        let mut s = flat(CHECK(), 40, 100.0);
        s.insert(CHECK(), 300.0);
        let f = evaluate_day(&s, CHECK(), &Rules::default()).expect("fires");
        assert_eq!(f.kind, AnomalyKind::Spike);
        assert!((f.expected - 100.0).abs() < 1e-9);
        assert!((f.delta - 200.0).abs() < 1e-9);
        assert_eq!(f.severity, Severity::High); // 200 × 30 = 6000
    }

    #[test]
    fn small_absolute_jump_never_fires() {
        let mut s = flat(CHECK(), 40, 0.10);
        s.insert(CHECK(), 1.00);
        assert_eq!(evaluate_day(&s, CHECK(), &Rules::default()), None);
    }

    #[test]
    fn small_relative_jump_never_fires() {
        let mut s = flat(CHECK(), 40, 10_000.0);
        s.insert(CHECK(), 10_050.0);
        assert_eq!(evaluate_day(&s, CHECK(), &Rules::default()), None);
    }

    #[test]
    fn flat_series_wobble_is_absorbed_by_spread_floor() {
        let rules = Rules {
            min_delta_usd: 0.0,
            min_delta_pct: 0.0,
            ..Rules::default()
        };
        let mut s = flat(CHECK(), 40, 200.0);
        s.insert(CHECK(), 205.0);
        assert_eq!(
            evaluate_day(&s, CHECK(), &rules),
            None,
            "5 < 3 × (0.05 × 200)"
        );
        s.insert(CHECK(), 240.0);
        assert!(evaluate_day(&s, CHECK(), &rules).is_some(), "40 ≥ 30");
    }

    #[test]
    fn weekly_pattern_is_expected_on_its_weekday() {
        let monday = d(2026, 9, 28);
        assert_eq!(monday.weekday(), Weekday::Mon);
        let mut s: BTreeMap<NaiveDate, f64> = (1..=42)
            .map(|i| {
                let day = monday - Duration::days(i);
                (
                    day,
                    if day.weekday() == Weekday::Mon {
                        250.0
                    } else {
                        100.0
                    },
                )
            })
            .collect();
        s.insert(monday, 250.0);
        assert_eq!(
            evaluate_day(&s, monday, &Rules::default()),
            None,
            "a normal Monday"
        );

        let tuesday = monday + Duration::days(1);
        s.insert(tuesday, 250.0);
        let f = evaluate_day(&s, tuesday, &Rules::default()).expect("Monday-sized Tuesday fires");
        assert_eq!(f.kind, AnomalyKind::Spike);
    }

    #[test]
    fn drops_fire_only_when_included() {
        let mut s = flat(CHECK(), 40, 100.0);
        s.insert(CHECK(), 20.0);
        assert_eq!(
            evaluate_day(&s, CHECK(), &Rules::default()),
            None,
            "drops are off by default"
        );
        let drops = Rules {
            include_drops: true,
            ..Rules::default()
        };
        assert_eq!(
            evaluate_day(&s, CHECK(), &drops).unwrap().kind,
            AnomalyKind::Drop
        );
    }

    #[test]
    fn missing_day_counts_as_zero_and_is_a_drop() {
        let s = flat(CHECK(), 40, 100.0); // nothing on CHECK
        let drops = Rules {
            include_drops: true,
            ..Rules::default()
        };
        assert_eq!(
            evaluate_day(&s, CHECK(), &drops).unwrap().kind,
            AnomalyKind::Drop
        );
    }

    #[test]
    fn new_service_fires_once_as_new_spend() {
        let start = d(2026, 9, 20);
        let s: BTreeMap<NaiveDate, f64> =
            (0..4).map(|i| (start + Duration::days(i), 50.0)).collect();
        let findings = detect(&s, start, start + Duration::days(3), &Rules::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].day, start);
        assert_eq!(findings[0].kind, AnomalyKind::NewSpend);
        assert!((findings[0].expected - 0.0).abs() < 1e-9);
    }

    #[test]
    fn new_spend_below_the_absolute_floor_is_ignored() {
        let s: BTreeMap<NaiveDate, f64> = [(CHECK(), 5.0)].into();
        assert_eq!(evaluate_day(&s, CHECK(), &Rules::default()), None);
    }

    #[test]
    fn short_history_with_existing_spend_does_not_fire() {
        let mut s = flat(CHECK(), 10, 50.0);
        s.insert(CHECK(), 52.0);
        assert_eq!(evaluate_day(&s, CHECK(), &Rules::default()), None);
    }

    #[test]
    fn earlier_spikes_in_the_baseline_do_not_mask_a_later_one() {
        let mut s = flat(CHECK(), 40, 100.0);
        s.insert(CHECK() - Duration::days(10), 500.0);
        s.insert(CHECK() - Duration::days(9), 500.0);
        s.insert(CHECK(), 400.0);
        assert!(evaluate_day(&s, CHECK(), &Rules::default()).is_some());
    }

    #[test]
    fn monthly_impact_rule_filters_when_set() {
        let rules = Rules {
            min_monthly_impact_usd: 1000.0,
            ..Rules::default()
        };
        let mut s = flat(CHECK(), 40, 100.0);
        s.insert(CHECK(), 130.0); // 30 × 30 = 900
        assert_eq!(evaluate_day(&s, CHECK(), &rules), None);
        s.insert(CHECK(), 140.0); // 40 × 30 = 1200
        assert!(evaluate_day(&s, CHECK(), &rules).is_some());
    }

    #[test]
    fn severity_band_boundaries() {
        let rules = Rules {
            severity_medium_usd: 300.0,
            severity_high_usd: 600.0,
            ..Rules::default()
        };
        assert_eq!(severity(9.99, &rules), Severity::Low);
        assert_eq!(severity(10.0, &rules), Severity::Medium);
        assert_eq!(severity(-20.0, &rules), Severity::High);
    }

    /// `base` per day from `start` through `end`, plus `lump` on the 1st of each month.
    fn base_with_monthly_lump(
        start: NaiveDate,
        end: NaiveDate,
        base: f64,
        lump: f64,
    ) -> BTreeMap<NaiveDate, f64> {
        start
            .iter_days()
            .take_while(|day| *day <= end)
            .map(|day| (day, base + if day.day() == 1 { lump } else { 0.0 }))
            .filter(|(_, cost)| *cost > 0.0)
            .collect()
    }

    #[test]
    fn monthly_lump_alone_fires_only_the_first_month() {
        let s = base_with_monthly_lump(d(2026, 8, 1), d(2026, 9, 30), 0.0, 300.0);
        let findings = detect(&s, d(2026, 8, 1), d(2026, 9, 30), &Rules::default());
        assert_eq!(
            findings.iter().map(|f| (f.day, f.kind)).collect::<Vec<_>>(),
            [(d(2026, 8, 1), AnomalyKind::NewSpend)],
            "September's lump echoes August's"
        );
    }

    #[test]
    fn monthly_lump_on_a_base_fires_only_the_first_month() {
        let s = base_with_monthly_lump(d(2026, 7, 2), d(2026, 9, 30), 5.0, 200.0);
        let findings = detect(&s, d(2026, 8, 1), d(2026, 9, 30), &Rules::default());
        assert_eq!(
            findings.iter().map(|f| (f.day, f.kind)).collect::<Vec<_>>(),
            [(d(2026, 8, 1), AnomalyKind::Spike)],
            "September's lump echoes August's"
        );
    }

    #[test]
    fn a_new_one_off_without_an_echo_still_fires() {
        let s: BTreeMap<NaiveDate, f64> = [(d(2026, 9, 15), 300.0)].into();
        let f = evaluate_day(&s, d(2026, 9, 15), &Rules::default()).expect("fires");
        assert_eq!((f.kind, f.actual), (AnomalyKind::NewSpend, 300.0));
    }

    #[test]
    fn a_lump_twice_last_months_still_fires() {
        let mut s = base_with_monthly_lump(d(2026, 7, 2), d(2026, 9, 30), 5.0, 200.0);
        s.insert(d(2026, 9, 1), 5.0 + 400.0);
        let f = evaluate_day(&s, d(2026, 9, 1), &Rules::default()).expect("fires");
        assert_eq!(f.kind, AnomalyKind::Spike);
        assert!((f.expected - 5.0).abs() < 1e-9);
    }

    /// $30 every 7 days before `check`: 4 days with data in a 28-day window.
    fn weekly_job(check: NaiveDate) -> BTreeMap<NaiveDate, f64> {
        (1..=4)
            .map(|w| (check - Duration::days(7 * w), 30.0))
            .collect()
    }

    #[test]
    fn sporadic_series_spike_fires_against_its_largest_day() {
        let mut s = weekly_job(CHECK());
        s.insert(CHECK(), 5000.0);
        let f = evaluate_day(&s, CHECK(), &Rules::default()).expect("fires");
        assert_eq!(f.kind, AnomalyKind::Spike);
        assert!((f.expected - 30.0).abs() < 1e-9);
        assert!((f.delta - 4970.0).abs() < 1e-9);
        assert_eq!(f.severity, Severity::High);
    }

    #[test]
    fn sporadic_series_small_rise_does_not_fire() {
        let mut s = weekly_job(CHECK());
        s.insert(CHECK(), 40.0);
        assert_eq!(
            evaluate_day(&s, CHECK(), &Rules::default()),
            None,
            "delta 10 < 20"
        );
    }

    #[test]
    fn spend_only_beyond_the_window_still_lets_a_spike_fire() {
        // Aug 1 is 31 days before Sep 1: outside the 28-day window but inside
        // new_spend's 35-day memory, so new_spend is blocked; the sporadic
        // rule must look just as far back.
        let mut s: BTreeMap<NaiveDate, f64> = [(d(2026, 8, 1), 30.0)].into();
        s.insert(d(2026, 9, 1), 5000.0);
        let f = evaluate_day(&s, d(2026, 9, 1), &Rules::default()).expect("fires");
        assert_eq!(f.kind, AnomalyKind::Spike);
        assert!((f.expected - 30.0).abs() < 1e-9);
        assert!((f.delta - 4970.0).abs() < 1e-9);

        s.insert(d(2026, 9, 1), 40.0);
        assert_eq!(evaluate_day(&s, d(2026, 9, 1), &Rules::default()), None, "delta 10 < 20");
        s.insert(d(2026, 9, 1), 30.0);
        assert_eq!(evaluate_day(&s, d(2026, 9, 1), &Rules::default()), None, "same charge as last month");
    }

    #[test]
    fn zero_lookback_finds_nothing_instead_of_panicking() {
        let mut s = flat(CHECK(), 40, 100.0);
        s.insert(CHECK(), 900.0);
        let rules = Rules {
            lookback_days: 0,
            ..Rules::default()
        };
        assert_eq!(evaluate_day(&s, CHECK(), &rules), None);
    }
}
