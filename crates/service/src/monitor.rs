//! One anomaly check of one source: fingerprint short-circuit, cost guard
//! estimate, daily-series query, detection per series, merge into the
//! inbox. Callers (the scheduler loop, Tauri commands, the HTTP harness)
//! decide when; this module decides what.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use data::config::{AnomalyConfig, CostGuardConfig};
use domain::anomaly::{detect, DailySeriesRow, ScopeKind, MONTH_ECHO_REACH_DAYS};
use serde::Serialize;

use crate::alerts::{AlertStore, RunOutcome, RunState, ScopedFindings};
use crate::cost::{cost_estimate, CostTier, EstimateRange, EstimateRequest};
use crate::registry::{SourceRegistry, SourceStatus};

/// Settled days every check re-evaluates (revised data updates them).
pub const RECHECK_DAYS: i64 = 7;
/// Most days one check evaluates after a long gap.
pub const MAX_CHECK_DAYS: i64 = 31;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckMode {
    /// Startup, after registration, the periodic tick: skipped when the data
    /// is unchanged or the scan would reach cost guard's hard limit.
    Auto,
    /// "Check now": always runs; over the hard limit it needs `confirmed`.
    Manual { confirmed: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckOutcome {
    Ok {
        checked_from: Option<NaiveDate>,
        checked_to: Option<NaiveDate>,
        new_anomalies: usize,
    },
    Unchanged,
    Disabled,
    AlreadyRunning,
    SkippedCostGuard {
        estimate_usd: f64,
    },
    NeedsConfirmation {
        estimate_usd: f64,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    pub source_id: String,
    #[serde(flatten)]
    pub outcome: CheckOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckWindow {
    pub query_start: NaiveDate,
    /// Exclusive.
    pub query_end: NaiveDate,
    pub check_from: NaiveDate,
}

/// What a check reads and evaluates. The newest evaluated day is decided
/// later from the data itself (`newest day with data − settling_days`).
pub fn check_window(
    today: NaiveDate,
    last_checked: Option<NaiveDate>,
    settling_days: u32,
    lookback_days: u32,
) -> CheckWindow {
    let latest = today - Duration::days(1 + i64::from(settling_days));
    let recent = latest - Duration::days(RECHECK_DAYS - 1);
    let oldest = latest - Duration::days(MAX_CHECK_DAYS - 1);
    let check_from = match last_checked {
        Some(day) => (day + Duration::days(1)).min(recent).max(oldest),
        None => recent,
    };
    // Detection reads the lookback and, for new-spend memory and the month
    // echo, up to MONTH_ECHO_REACH_DAYS back; 2 more days of slack.
    let reach = i64::from(lookback_days).max(MONTH_ECHO_REACH_DAYS) + 2;
    CheckWindow {
        query_start: check_from - Duration::days(reach),
        query_end: today + Duration::days(1),
        check_from,
    }
}

/// Stable hash of every rule that affects detection: everything in
/// `AnomalyConfig` except `enabled` and `check_interval_hours`, overrides included.
pub fn rules_fingerprint(config: &AnomalyConfig) -> String {
    let projection = AnomalyConfig {
        enabled: true,
        check_interval_hours: 0,
        ..config.clone()
    };
    let mut hasher = crate::register::Fnv1a::default();
    // serde_json keeps struct field order, so the encoding is stable.
    hasher.feed(
        serde_json::to_string(&projection)
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.hex()
}

fn group_series(
    rows: Vec<DailySeriesRow>,
) -> BTreeMap<(ScopeKind, String), BTreeMap<NaiveDate, f64>> {
    let mut out: BTreeMap<(ScopeKind, String), BTreeMap<NaiveDate, f64>> = BTreeMap::new();
    for r in rows {
        *out.entry((r.scope_kind, r.scope_key))
            .or_default()
            .entry(r.day)
            .or_insert(0.0) += r.cost;
    }
    out
}

pub struct CheckContext<'a> {
    pub registry: &'a SourceRegistry,
    pub guard: &'a CostGuardConfig,
    pub config: &'a AnomalyConfig,
    pub store: &'a AlertStore,
    pub treat_local_as_remote: bool,
}

/// Tracks which sources have a check in flight; a second trigger for the
/// same source is dropped (`AlreadyRunning`) rather than queued.
#[derive(Default)]
pub struct Monitor {
    running: Mutex<HashSet<String>>,
}

pub(crate) struct Running<'a> {
    monitor: &'a Monitor,
    id: String,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.monitor
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

impl Monitor {
    pub(crate) fn try_start(&self, id: &str) -> Option<Running<'_>> {
        let inserted = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string());
        inserted.then(|| Running {
            monitor: self,
            id: id.to_string(),
        })
    }

    pub fn check(
        &self,
        ctx: &CheckContext,
        source_id: &str,
        mode: CheckMode,
        now: DateTime<Utc>,
    ) -> CheckResult {
        CheckResult {
            source_id: source_id.to_string(),
            outcome: self.run(ctx, source_id, mode, now),
        }
    }

    fn run(
        &self,
        ctx: &CheckContext,
        source_id: &str,
        mode: CheckMode,
        now: DateTime<Utc>,
    ) -> CheckOutcome {
        if !ctx.config.enabled {
            return CheckOutcome::Disabled;
        }
        let Some(_running) = self.try_start(source_id) else {
            return CheckOutcome::AlreadyRunning;
        };
        let previous = ctx.store.run_state(source_id);
        let fail = |message: String, estimate_usd: Option<f64>| {
            let run = RunState {
                fingerprint: previous.as_ref().and_then(|r| r.fingerprint.clone()),
                last_checked_day: previous.as_ref().and_then(|r| r.last_checked_day),
                last_run_at: now,
                outcome: RunOutcome::Error {
                    message: message.clone(),
                },
                estimate_usd,
            };
            if let Err(e) = ctx.store.record_run(source_id, run) {
                tracing::warn!(source_id, error = %e, "could not record failed anomaly check");
            }
            CheckOutcome::Error { message }
        };
        if let Err(message) = ctx.config.validate() {
            return fail(message, None);
        }
        let scan = match ctx.registry.scan(Some(source_id)) {
            Ok(scan) => scan,
            Err(e) => return CheckOutcome::Error { message: e.message },
        };
        // Covers data and rules, so a saved rule change re-runs the automatic check.
        let fingerprint = format!("{}:{}", scan.fingerprint, rules_fingerprint(ctx.config));
        // Only a completed (or cost-guard-skipped) run counts: after an error the next tick retries.
        let unchanged = previous.as_ref().is_some_and(|r| {
            r.fingerprint.as_deref() == Some(fingerprint.as_str())
                && matches!(
                    r.outcome,
                    RunOutcome::Ok | RunOutcome::SkippedCostGuard { .. }
                )
        });
        if mode == CheckMode::Auto && unchanged {
            return CheckOutcome::Unchanged;
        }

        let today = now.date_naive();
        let last_checked = previous.as_ref().and_then(|r| r.last_checked_day);
        let window = check_window(
            today,
            last_checked,
            ctx.config.settling_days,
            ctx.config.lookback_days,
        );

        // Fails open, like the page-level cost guard: an estimate error never blocks a check.
        let estimate = cost_estimate(
            ctx.registry,
            ctx.guard,
            ctx.treat_local_as_remote,
            EstimateRequest {
                source_id: Some(source_id.to_string()),
                scans: vec![vec![EstimateRange {
                    start: window.query_start,
                    end: window.query_end,
                }]],
            },
        )
        .ok();
        let estimate_usd = estimate
            .as_ref()
            .filter(|e| e.remote && e.known)
            .map(|e| e.usd);
        if let Some(e) = estimate.as_ref().filter(|e| e.tier == CostTier::Hard) {
            match mode {
                CheckMode::Auto => {
                    let run = RunState {
                        fingerprint: previous.as_ref().and_then(|r| r.fingerprint.clone()),
                        last_checked_day: last_checked,
                        last_run_at: now,
                        outcome: RunOutcome::SkippedCostGuard {
                            estimate_usd: e.usd,
                        },
                        estimate_usd: Some(e.usd),
                    };
                    if let Err(err) = ctx.store.record_run(source_id, run) {
                        tracing::warn!(source_id, error = %err, "could not record skipped anomaly check");
                    }
                    return CheckOutcome::SkippedCostGuard {
                        estimate_usd: e.usd,
                    };
                }
                CheckMode::Manual { confirmed: false } => {
                    return CheckOutcome::NeedsConfirmation {
                        estimate_usd: e.usd,
                    }
                }
                CheckMode::Manual { confirmed: true } => {}
            }
        }

        // A registration with no files has no `normalized_cost` view to query: nothing to find.
        let empty = ctx.registry.diagnostics().iter().any(|d| {
            d.id == source_id && matches!(d.status, SourceStatus::Registered { file_count: 0, .. })
        });
        if empty {
            let run = RunState {
                fingerprint: Some(fingerprint.clone()),
                last_checked_day: last_checked,
                last_run_at: now,
                outcome: RunOutcome::Ok,
                estimate_usd,
            };
            return match ctx.store.apply_run(
                source_id,
                (window.check_from, window.check_from - Duration::days(1)),
                &[],
                run,
                today,
                now,
            ) {
                Ok(new_anomalies) => CheckOutcome::Ok {
                    checked_from: Some(window.check_from),
                    checked_to: None,
                    new_anomalies,
                },
                Err(e) => CheckOutcome::Error { message: e.message },
            };
        }

        let rows = match ctx
            .registry
            .repo(Some(source_id))
            .map_err(|e| e.message)
            .and_then(|repo| {
                repo.daily_series(window.query_start, window.query_end, ctx.config.metric)
                    .map_err(|e| e.to_string())
            }) {
            Ok(rows) => rows,
            Err(message) => return fail(message, estimate_usd),
        };

        let checked_to = rows
            .iter()
            .map(|r| r.day)
            .max()
            .map(|d| d - Duration::days(i64::from(ctx.config.settling_days)));
        let mut scoped = Vec::new();
        if let Some(to) = checked_to.filter(|to| *to >= window.check_from) {
            for ((scope_kind, scope_key), series) in group_series(rows) {
                let rules = ctx.config.rules_for(scope_kind, &scope_key);
                let findings = detect(&series, window.check_from, to, &rules);
                if !findings.is_empty() {
                    scoped.push(ScopedFindings {
                        scope_kind,
                        scope_key,
                        findings,
                    });
                }
            }
        }
        let run = RunState {
            fingerprint: Some(fingerprint.clone()),
            last_checked_day: checked_to.max(last_checked),
            last_run_at: now,
            outcome: RunOutcome::Ok,
            estimate_usd,
        };
        // Empty range (`from > to`) when no settled day was evaluated: nothing is retracted.
        let checked = (
            window.check_from,
            checked_to.unwrap_or(window.check_from - Duration::days(1)),
        );
        match ctx
            .store
            .apply_run(source_id, checked, &scoped, run, today, now)
        {
            Ok(new_anomalies) => CheckOutcome::Ok {
                checked_from: Some(window.check_from),
                checked_to,
                new_anomalies,
            },
            Err(e) => CheckOutcome::Error { message: e.message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::{AlertStore, RunOutcome};
    use crate::register::{register_source, Registered};
    use chrono::TimeZone;
    use data::config::{AnomalyConfig, CostGuardConfig, DataSource};
    use data::fixtures::{
        generate_anomaly_fixture, ANOMALY_NEW_ACCOUNT, ANOMALY_NEW_SERVICE, ANOMALY_SPIKE_ACCOUNT,
        ANOMALY_SPIKE_SERVICE,
    };
    use domain::anomaly::{AnomalyKind, ScopeKind};

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }
    /// The fixture ends 2026-09-30; checks run the next day.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }

    struct Env {
        _dir: tempfile::TempDir,
        registry: SourceRegistry,
        store: AlertStore,
        guard: CostGuardConfig,
        config: AnomalyConfig,
        remote: bool,
    }

    impl Env {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            generate_anomaly_fixture(dir.path(), d(9, 30)).unwrap();
            let source = DataSource {
                id: "a".into(),
                name: "A".into(),
                s3_uri: dir.path().to_str().unwrap().into(),
                ..Default::default()
            };
            let registry = SourceRegistry::new();
            registry.mark_pending(&source);
            registry.set_result("a", register_source(&source, None));
            Env {
                _dir: dir,
                registry,
                store: AlertStore::in_memory(),
                guard: CostGuardConfig::default(),
                config: AnomalyConfig::default(),
                remote: false,
            }
        }
        fn ctx(&self) -> CheckContext<'_> {
            CheckContext {
                registry: &self.registry,
                guard: &self.guard,
                config: &self.config,
                store: &self.store,
                treat_local_as_remote: self.remote,
            }
        }
    }

    #[test]
    fn window_covers_seven_settled_days_then_resumes_after_last_checked() {
        let w = check_window(d(10, 1), None, 2, 28);
        assert_eq!(
            (w.check_from, w.query_start, w.query_end),
            (d(9, 22), d(8, 16), d(10, 2)),
            "max(28, 35) + 2 days before check_from"
        );
        assert_eq!(
            check_window(d(10, 1), None, 2, 60).query_start,
            d(7, 22),
            "a lookback over 35 days reaches lookback + 2"
        );
        assert_eq!(
            check_window(d(10, 1), Some(d(9, 27)), 2, 28).check_from,
            d(9, 22),
            "always re-checks the last 7"
        );
        assert_eq!(
            check_window(d(10, 1), Some(d(9, 10)), 2, 28).check_from,
            d(9, 11)
        );
        assert_eq!(
            check_window(d(10, 1), Some(d(1, 1)), 2, 28).check_from,
            d(8, 29),
            "at most 31 days"
        );
    }

    #[test]
    fn first_check_finds_exactly_the_fixture_anomalies() {
        let env = Env::new();
        let r = Monitor::default().check(&env.ctx(), "a", CheckMode::Auto, now());
        assert_eq!(
            r.outcome,
            CheckOutcome::Ok {
                checked_from: Some(d(9, 22)),
                checked_to: Some(d(9, 28)),
                new_anomalies: 4
            }
        );

        let snap = env.store.snapshot(now().date_naive());
        let mut found: Vec<(ScopeKind, &str, AnomalyKind)> = snap
            .anomalies
            .iter()
            .map(|a| (a.scope_kind, a.scope_key.as_str(), a.kind))
            .collect();
        found.sort_by_key(|f| (f.0, f.1.to_string()));
        assert_eq!(
            found,
            vec![
                (
                    ScopeKind::Service,
                    ANOMALY_SPIKE_SERVICE,
                    AnomalyKind::Spike
                ),
                (
                    ScopeKind::Service,
                    ANOMALY_NEW_SERVICE,
                    AnomalyKind::NewSpend
                ),
                (
                    ScopeKind::Account,
                    ANOMALY_SPIKE_ACCOUNT,
                    AnomalyKind::Spike
                ),
                (
                    ScopeKind::Account,
                    ANOMALY_NEW_ACCOUNT,
                    AnomalyKind::NewSpend
                ),
            ]
        );
        let ec2 = snap
            .anomalies
            .iter()
            .find(|a| a.scope_key == ANOMALY_SPIKE_SERVICE)
            .unwrap();
        assert_eq!(
            (ec2.first_day, ec2.last_day),
            (d(9, 25), d(9, 27)),
            "unsettled 9/29–30 never evaluated"
        );
        let run = env.store.run_state("a").unwrap();
        assert_eq!(
            (run.outcome, run.last_checked_day),
            (RunOutcome::Ok, Some(d(9, 28)))
        );
    }

    #[test]
    fn unchanged_data_skips_auto_but_manual_re_evaluates_without_duplicates() {
        let env = Env::new();
        let monitor = Monitor::default();
        monitor.check(&env.ctx(), "a", CheckMode::Auto, now());
        assert_eq!(
            monitor
                .check(&env.ctx(), "a", CheckMode::Auto, now())
                .outcome,
            CheckOutcome::Unchanged
        );
        let manual = monitor.check(
            &env.ctx(),
            "a",
            CheckMode::Manual { confirmed: false },
            now(),
        );
        assert!(
            matches!(
                manual.outcome,
                CheckOutcome::Ok {
                    new_anomalies: 0,
                    ..
                }
            ),
            "{manual:?}"
        );
        assert_eq!(env.store.snapshot(now().date_naive()).anomalies.len(), 4);
    }

    #[test]
    fn changed_rules_re_run_auto_but_scheduling_fields_do_not() {
        let mut env = Env::new();
        let monitor = Monitor::default();
        monitor.check(&env.ctx(), "a", CheckMode::Auto, now());
        env.config.check_interval_hours = 24;
        assert_eq!(
            monitor
                .check(&env.ctx(), "a", CheckMode::Auto, now())
                .outcome,
            CheckOutcome::Unchanged,
            "the interval doesn't affect detection"
        );
        env.config.min_delta_usd = 50.0;
        let changed = monitor.check(&env.ctx(), "a", CheckMode::Auto, now());
        assert!(
            matches!(changed.outcome, CheckOutcome::Ok { .. }),
            "{changed:?}"
        );
        assert_eq!(
            monitor
                .check(&env.ctx(), "a", CheckMode::Auto, now())
                .outcome,
            CheckOutcome::Unchanged
        );
        env.config.overrides.push(data::config::AnomalyOverride {
            scope: ScopeKind::Service,
            key: ANOMALY_SPIKE_SERVICE.into(),
            mad_multiplier: None,
            min_delta_usd: Some(5000.0),
            min_delta_pct: None,
            min_monthly_impact_usd: None,
            include_drops: None,
            severity_medium_usd: None,
            severity_high_usd: None,
        });
        assert!(matches!(
            monitor
                .check(&env.ctx(), "a", CheckMode::Auto, now())
                .outcome,
            CheckOutcome::Ok { .. }
        ));
    }

    #[test]
    fn a_failed_check_is_retried_even_with_a_matching_fingerprint() {
        let env = Env::new();
        let monitor = Monitor::default();
        monitor.check(&env.ctx(), "a", CheckMode::Auto, now());
        let ok = env.store.run_state("a").unwrap();
        env.store
            .record_run(
                "a",
                RunState {
                    outcome: RunOutcome::Error {
                        message: "disk full".into(),
                    },
                    ..ok
                },
            )
            .unwrap();
        let retried = monitor.check(&env.ctx(), "a", CheckMode::Auto, now());
        assert!(
            matches!(retried.outcome, CheckOutcome::Ok { .. }),
            "{retried:?}"
        );
        assert_eq!(env.store.run_state("a").unwrap().outcome, RunOutcome::Ok);
    }

    #[test]
    fn hard_limit_skips_auto_and_asks_before_manual() {
        let mut env = Env::new();
        env.remote = true;
        env.guard = CostGuardConfig {
            soft_limit_usd: 0.0,
            hard_limit_usd: 0.0,
            ..Default::default()
        };
        let monitor = Monitor::default();
        let auto = monitor.check(&env.ctx(), "a", CheckMode::Auto, now());
        assert!(
            matches!(auto.outcome, CheckOutcome::SkippedCostGuard { .. }),
            "{auto:?}"
        );
        assert!(matches!(
            env.store.run_state("a").unwrap().outcome,
            RunOutcome::SkippedCostGuard { .. }
        ));
        assert!(env.store.snapshot(now().date_naive()).anomalies.is_empty());

        let ask = monitor.check(
            &env.ctx(),
            "a",
            CheckMode::Manual { confirmed: false },
            now(),
        );
        assert!(
            matches!(ask.outcome, CheckOutcome::NeedsConfirmation { .. }),
            "{ask:?}"
        );
        let go = monitor.check(
            &env.ctx(),
            "a",
            CheckMode::Manual { confirmed: true },
            now(),
        );
        assert!(
            matches!(
                go.outcome,
                CheckOutcome::Ok {
                    new_anomalies: 4,
                    ..
                }
            ),
            "{go:?}"
        );
        assert!(env.store.run_state("a").unwrap().estimate_usd.unwrap() > 0.0);
    }

    #[test]
    fn query_errors_are_recorded_and_isolated_per_source() {
        let env = Env::new();
        let broken = DataSource {
            id: "b".into(),
            name: "B".into(),
            s3_uri: "x".into(),
            ..Default::default()
        };
        env.registry.mark_pending(&broken);
        let empty_pool = data::duckdb_pool::build_pool().unwrap(); // no normalized_cost view
        env.registry.set_result(
            "b",
            Ok(Registered {
                repo: std::sync::Arc::new(data::queries::summary::DuckDbCostRepository::new(
                    empty_pool,
                )),
                detected_format: "focus12".into(),
                file_count: 1,
                billing_periods: vec![],
                remote: false,
                scan_index: None,
                fingerprint: "b-fp".into(),
            }),
        );
        let monitor = Monitor::default();
        let r = monitor.check(&env.ctx(), "b", CheckMode::Auto, now());
        assert!(matches!(r.outcome, CheckOutcome::Error { .. }), "{r:?}");
        let run = env.store.run_state("b").unwrap();
        assert!(matches!(run.outcome, RunOutcome::Error { .. }));
        assert_eq!(
            run.fingerprint, None,
            "a failed check is retried on the next tick"
        );
        assert!(matches!(
            monitor
                .check(&env.ctx(), "a", CheckMode::Auto, now())
                .outcome,
            CheckOutcome::Ok { .. }
        ));
    }

    #[test]
    fn empty_source_checks_ok_with_nothing_found() {
        let env = Env::new();
        let dir = tempfile::tempdir().unwrap();
        let empty = DataSource {
            id: "e".into(),
            name: "E".into(),
            s3_uri: dir.path().to_str().unwrap().into(),
            ..Default::default()
        };
        env.registry.mark_pending(&empty);
        env.registry.set_result("e", register_source(&empty, None));
        let r = Monitor::default().check(&env.ctx(), "e", CheckMode::Auto, now());
        assert_eq!(
            r.outcome,
            CheckOutcome::Ok {
                checked_from: Some(d(9, 22)),
                checked_to: None,
                new_anomalies: 0
            }
        );
    }

    #[test]
    fn invalid_hand_edited_rules_record_an_error_instead_of_panicking() {
        let mut env = Env::new();
        env.config.lookback_days = 0;
        let r = Monitor::default().check(&env.ctx(), "a", CheckMode::Auto, now());
        assert_eq!(
            r.outcome,
            CheckOutcome::Error {
                message: "lookback_days must be between 14 and 90".into()
            }
        );
        assert!(matches!(
            env.store.run_state("a").unwrap().outcome,
            RunOutcome::Error { .. }
        ));
    }

    #[test]
    fn disabled_and_already_running() {
        let mut env = Env::new();
        let monitor = Monitor::default();
        let _held = monitor.try_start("a").unwrap();
        assert_eq!(
            monitor
                .check(&env.ctx(), "a", CheckMode::Auto, now())
                .outcome,
            CheckOutcome::AlreadyRunning
        );
        env.config.enabled = false;
        assert_eq!(
            monitor
                .check(
                    &env.ctx(),
                    "a",
                    CheckMode::Manual { confirmed: true },
                    now()
                )
                .outcome,
            CheckOutcome::Disabled
        );
    }
}
