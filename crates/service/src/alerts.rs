//! The anomaly inbox: `alerts.json` next to `settings.toml`. Holds detected
//! anomalies (consecutive firing days of one scope merged into one), mutes
//! and each source's last check. Every change is a read-modify-write under
//! one mutex, persisted atomically before it's committed in memory.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::anomaly::{AnomalyKind, Direction, Finding, ScopeKind, Severity};
use serde::{Deserialize, Serialize};

use crate::error::ServiceError;

pub const ALERTS_FILE_VERSION: u32 = 1;
/// Non-firing days that may separate two findings of the same anomaly.
pub const MAX_GAP_DAYS: i64 = 2;
/// Anomalies whose last day is older than this are pruned on load.
pub const RETENTION_DAYS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyStatus {
    Open,
    Acknowledged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyPoint {
    pub day: NaiveDate,
    pub expected: f64,
    pub actual: f64,
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anomaly {
    pub id: String,
    pub source_id: String,
    pub scope_kind: ScopeKind,
    pub scope_key: String,
    pub kind: AnomalyKind,
    pub direction: Direction,
    pub first_day: NaiveDate,
    pub last_day: NaiveDate,
    /// Sorted by day, one per firing day.
    pub points: Vec<AnomalyPoint>,
    /// Sum of `actual - expected` over the points.
    pub total_impact: f64,
    /// Highest point severity.
    pub severity: Severity,
    pub status: AnomalyStatus,
    /// Reopened because its severity rose after it was acknowledged; cleared on acknowledge.
    #[serde(default)]
    pub escalated: bool,
    /// Derived from live mutes (see [`apply_mutes`]); stored for readers of the file.
    pub muted: bool,
    pub detected_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mute {
    /// Mutes are per source: the same service in another source stays visible.
    pub source_id: String,
    pub scope_kind: ScopeKind,
    pub scope_key: String,
    /// Inclusive.
    pub until: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunOutcome {
    Ok,
    SkippedCostGuard { estimate_usd: f64 },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    /// Data fingerprint the last successful check saw; `None` until one succeeds.
    pub fingerprint: Option<String>,
    pub last_checked_day: Option<NaiveDate>,
    pub last_run_at: DateTime<Utc>,
    pub outcome: RunOutcome,
    /// Cost guard's estimate for the check's scan (remote sources only).
    #[serde(default)]
    pub estimate_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    pub version: u32,
    #[serde(default)]
    pub anomalies: Vec<Anomaly>,
    #[serde(default)]
    pub mutes: Vec<Mute>,
    #[serde(default)]
    pub runs: BTreeMap<String, RunState>,
}

impl Default for AlertState {
    fn default() -> Self {
        Self { version: ALERTS_FILE_VERSION, anomalies: Vec::new(), mutes: Vec::new(), runs: BTreeMap::new() }
    }
}

/// One series' findings from one check.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedFindings {
    pub scope_kind: ScopeKind,
    pub scope_key: String,
    pub findings: Vec<Finding>,
}

pub fn merge_findings(anomalies: &mut Vec<Anomaly>, source_id: &str, scoped: &ScopedFindings, now: DateTime<Utc>) -> usize {
    let reach = Duration::days(MAX_GAP_DAYS + 1);
    let mut findings = scoped.findings.clone();
    findings.sort_by_key(|f| f.day);
    let mut created = 0;
    for f in &findings {
        let direction = f.kind.direction();
        let point = AnomalyPoint { day: f.day, expected: f.expected, actual: f.actual, severity: f.severity };
        let existing = anomalies.iter_mut().find(|a| {
            a.source_id == source_id
                && a.scope_kind == scoped.scope_kind
                && a.scope_key == scoped.scope_key
                && a.direction == direction
                && f.day >= a.first_day - reach
                && f.day <= a.last_day + reach
        });
        match existing {
            Some(a) => {
                let before = a.severity;
                match a.points.iter_mut().find(|p| p.day == f.day) {
                    Some(p) => *p = point,
                    None => {
                        a.points.push(point);
                        a.points.sort_by_key(|p| p.day);
                    }
                }
                recompute(a);
                if a.status == AnomalyStatus::Acknowledged && a.severity > before {
                    a.status = AnomalyStatus::Open;
                    a.escalated = true;
                }
                a.updated_at = now;
            }
            None => {
                let mut a = Anomaly {
                    id: format!("{source_id}:{}:{}:{}:{}", scoped.scope_kind.as_str(), scoped.scope_key, direction.as_str(), f.day),
                    source_id: source_id.to_string(),
                    scope_kind: scoped.scope_kind,
                    scope_key: scoped.scope_key.clone(),
                    kind: f.kind,
                    direction,
                    first_day: f.day,
                    last_day: f.day,
                    points: vec![point],
                    total_impact: 0.0,
                    severity: f.severity,
                    status: AnomalyStatus::Open,
                    escalated: false,
                    muted: false,
                    detected_at: now,
                    updated_at: now,
                };
                recompute(&mut a);
                anomalies.push(a);
                created += 1;
            }
        }
    }
    created
}

fn recompute(a: &mut Anomaly) {
    a.first_day = a.points.first().map_or(a.first_day, |p| p.day);
    a.last_day = a.points.last().map_or(a.last_day, |p| p.day);
    a.total_impact = a.points.iter().map(|p| p.actual - p.expected).sum();
    a.severity = a.points.iter().map(|p| p.severity).max().unwrap_or(a.severity);
}

/// Drops points in `checked` (inclusive) of this source's **open** anomalies
/// that the check no longer finds, because AWS revised the data; an open
/// anomaly left without points is removed. Acknowledged anomalies stay as
/// the user saw them. An empty range (`from > to`) retracts nothing.
pub fn retract_stale(
    anomalies: &mut Vec<Anomaly>,
    source_id: &str,
    checked: (NaiveDate, NaiveDate),
    scoped: &[ScopedFindings],
    now: DateTime<Utc>,
) {
    let (from, to) = checked;
    for a in anomalies.iter_mut().filter(|a| a.source_id == source_id && a.status == AnomalyStatus::Open) {
        let fired: Vec<NaiveDate> = scoped
            .iter()
            .filter(|sc| sc.scope_kind == a.scope_kind && sc.scope_key == a.scope_key)
            .flat_map(|sc| sc.findings.iter())
            .filter(|f| f.kind.direction() == a.direction)
            .map(|f| f.day)
            .collect();
        let before = a.points.len();
        a.points.retain(|p| p.day < from || p.day > to || fired.contains(&p.day));
        if a.points.len() != before && !a.points.is_empty() {
            recompute(a);
            a.updated_at = now;
        }
    }
    anomalies.retain(|a| !a.points.is_empty());
}

pub fn apply_mutes(state: &mut AlertState, today: NaiveDate) {
    state.mutes.retain(|m| m.until >= today);
    let mutes = &state.mutes;
    for a in state.anomalies.iter_mut() {
        a.muted = mutes
            .iter()
            .any(|m| m.source_id == a.source_id && m.scope_kind == a.scope_kind && m.scope_key == a.scope_key);
    }
}

pub fn prune(state: &mut AlertState, today: NaiveDate) {
    let cutoff = today - Duration::days(RETENTION_DAYS);
    state.anomalies.retain(|a| a.last_day >= cutoff);
}

pub struct AlertStore {
    /// `None` = in memory only (tests, the non-persistent HTTP harness).
    path: Option<PathBuf>,
    state: Mutex<AlertState>,
}

impl AlertStore {
    pub fn in_memory() -> Self {
        Self { path: None, state: Mutex::new(AlertState::default()) }
    }

    /// Loads `path` (missing = empty; corrupt or unknown version =
    /// quarantined and empty), prunes and applies mutes.
    pub fn open(path: PathBuf, today: NaiveDate) -> Self {
        let mut state = load(&path);
        prune(&mut state, today);
        apply_mutes(&mut state, today);
        Self { path: Some(path), state: Mutex::new(state) }
    }

    fn lock(&self) -> MutexGuard<'_, AlertState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Applies `f` to a copy, persists it, then commits it; a failed write
    /// leaves memory unchanged. Holding the lock throughout serializes
    /// concurrent checks, acknowledgements and mutes.
    fn update<T>(&self, f: impl FnOnce(&mut AlertState) -> Result<T, ServiceError>) -> Result<T, ServiceError> {
        let mut guard = self.lock();
        let mut next = guard.clone();
        let out = f(&mut next)?;
        if let Some(path) = &self.path {
            write_atomic(path, &next)?;
        }
        *guard = next;
        Ok(out)
    }

    pub fn snapshot(&self, today: NaiveDate) -> AlertState {
        let mut state = self.lock().clone();
        apply_mutes(&mut state, today);
        state
    }

    pub fn run_state(&self, source_id: &str) -> Option<RunState> {
        self.lock().runs.get(source_id).cloned()
    }

    /// Retracts points in `checked` that no longer fire, merges one check's
    /// findings and records its run; returns anomalies created.
    pub fn apply_run(
        &self,
        source_id: &str,
        checked: (NaiveDate, NaiveDate),
        scoped: &[ScopedFindings],
        run: RunState,
        today: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<usize, ServiceError> {
        self.update(|s| {
            retract_stale(&mut s.anomalies, source_id, checked, scoped, now);
            let created = scoped.iter().map(|sc| merge_findings(&mut s.anomalies, source_id, sc, now)).sum();
            s.runs.insert(source_id.to_string(), run);
            apply_mutes(s, today);
            Ok(created)
        })
    }

    /// Records a run that produced no findings to merge (skipped or failed).
    pub fn record_run(&self, source_id: &str, run: RunState) -> Result<(), ServiceError> {
        self.update(|s| {
            s.runs.insert(source_id.to_string(), run);
            Ok(())
        })
    }

    pub fn acknowledge(&self, id: &str, now: DateTime<Utc>) -> Result<Anomaly, ServiceError> {
        self.update(|s| {
            let a = s.anomalies.iter_mut().find(|a| a.id == id).ok_or_else(|| ServiceError::not_found(format!("unknown anomaly '{id}'")))?;
            a.status = AnomalyStatus::Acknowledged;
            a.escalated = false;
            a.updated_at = now;
            Ok(a.clone())
        })
    }

    /// Mutes one source's scope through `until` inclusive, replacing any existing mute for it.
    pub fn mute(&self, source_id: &str, scope_kind: ScopeKind, scope_key: &str, until: NaiveDate, today: NaiveDate) -> Result<(), ServiceError> {
        self.update(|s| {
            s.mutes.retain(|m| !(m.source_id == source_id && m.scope_kind == scope_kind && m.scope_key == scope_key));
            s.mutes.push(Mute { source_id: source_id.to_string(), scope_kind, scope_key: scope_key.to_string(), until });
            apply_mutes(s, today);
            Ok(())
        })
    }

    /// Drops a deleted source's anomalies and run state.
    pub fn forget_source(&self, source_id: &str) -> Result<(), ServiceError> {
        self.update(|s| {
            s.anomalies.retain(|a| a.source_id != source_id);
            s.mutes.retain(|m| m.source_id != source_id);
            s.runs.remove(source_id);
            Ok(())
        })
    }
}

fn load(path: &std::path::Path) -> AlertState {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AlertState::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not read alerts file; starting empty");
            return AlertState::default();
        }
    };
    match serde_json::from_str::<AlertState>(&text) {
        Ok(state) if state.version == ALERTS_FILE_VERSION => state,
        Ok(state) => {
            quarantine(path, &format!("unknown version {}", state.version));
            AlertState::default()
        }
        Err(e) => {
            quarantine(path, &e.to_string());
            AlertState::default()
        }
    }
}

/// Keeps an unreadable alerts file for inspection instead of crashing or silently overwriting it.
fn quarantine(path: &std::path::Path, reason: &str) {
    let target = path.with_file_name(format!("alerts.corrupt-{}.json", Utc::now().format("%Y%m%dT%H%M%S")));
    match std::fs::rename(path, &target) {
        Ok(()) => tracing::warn!(path = %path.display(), moved_to = %target.display(), %reason, "alerts file unreadable; starting empty"),
        Err(e) => tracing::warn!(path = %path.display(), error = %e, %reason, "alerts file unreadable and could not be moved aside"),
    }
}

/// Same temp-file-then-rename pattern as `AppConfig::save`.
fn write_atomic(path: &std::path::Path, state: &AlertState) -> Result<(), ServiceError> {
    let text = serde_json::to_string_pretty(state).map_err(|e| ServiceError::internal(format!("failed to encode alerts: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| ServiceError::internal(format!("failed to save alerts: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    }
    fn finding(day: u32, delta: f64) -> Finding {
        let kind = if delta >= 0.0 { AnomalyKind::Spike } else { AnomalyKind::Drop };
        Finding { day: d(day), expected: 100.0, actual: 100.0 + delta, delta, kind, severity: if delta.abs() > 50.0 { Severity::High } else { Severity::Low } }
    }
    fn scoped(key: &str, findings: Vec<Finding>) -> ScopedFindings {
        ScopedFindings { scope_kind: ScopeKind::Service, scope_key: key.into(), findings }
    }
    fn run(day: u32) -> RunState {
        RunState { fingerprint: Some("fp".into()), last_checked_day: Some(d(day)), last_run_at: now(), outcome: RunOutcome::Ok, estimate_usd: None }
    }

    #[test]
    fn consecutive_and_near_days_merge_and_a_long_gap_splits() {
        let mut list = Vec::new();
        assert_eq!(merge_findings(&mut list, "s", &scoped("EC2", vec![finding(10, 30.0), finding(11, 80.0)]), now()), 1);
        assert_eq!(merge_findings(&mut list, "s", &scoped("EC2", vec![finding(14, 20.0)]), now()), 0, "gap of 2 days continues");
        assert_eq!(merge_findings(&mut list, "s", &scoped("EC2", vec![finding(18, 20.0)]), now()), 1, "gap of 3 days splits");
        assert_eq!(list.len(), 2);
        let a = &list[0];
        assert_eq!((a.first_day, a.last_day, a.points.len()), (d(10), d(14), 3));
        assert!((a.total_impact - 130.0).abs() < 1e-9);
        assert_eq!(a.severity, Severity::High);
        assert_eq!(a.id, "s:service:EC2:up:2026-09-10");
    }

    #[test]
    fn re_merging_the_same_day_updates_instead_of_duplicating() {
        let mut list = Vec::new();
        merge_findings(&mut list, "s", &scoped("EC2", vec![finding(10, 30.0)]), now());
        merge_findings(&mut list, "s", &scoped("EC2", vec![finding(10, 60.0)]), now());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].points.len(), 1);
        assert!((list[0].total_impact - 60.0).abs() < 1e-9);
    }

    #[test]
    fn directions_and_scopes_stay_separate() {
        let mut list = Vec::new();
        merge_findings(&mut list, "s", &scoped("EC2", vec![finding(10, 30.0), finding(11, -30.0)]), now());
        merge_findings(&mut list, "s", &scoped("S3", vec![finding(10, 30.0)]), now());
        merge_findings(&mut list, "other", &scoped("EC2", vec![finding(10, 30.0)]), now());
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn acknowledged_anomaly_stays_acknowledged_when_extended() {
        let store = AlertStore::in_memory();
        store.apply_run("s", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
        let id = store.snapshot(today()).anomalies[0].id.clone();
        store.acknowledge(&id, now()).unwrap();
        store.apply_run("s", (d(11), d(11)), &[scoped("EC2", vec![finding(11, 30.0)])], run(11), today(), now()).unwrap();
        let a = &store.snapshot(today()).anomalies[0];
        assert_eq!((a.status, a.points.len(), a.escalated), (AnomalyStatus::Acknowledged, 2, false));
        assert_eq!(store.acknowledge("nope", now()).unwrap_err().kind, crate::ErrorKind::NotFound);
    }

    #[test]
    fn acknowledged_anomaly_reopens_when_its_severity_rises() {
        let store = AlertStore::in_memory();
        store.apply_run("s", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
        let id = store.snapshot(today()).anomalies[0].id.clone();
        store.acknowledge(&id, now()).unwrap();
        store.apply_run("s", (d(11), d(11)), &[scoped("EC2", vec![finding(11, 80.0)])], run(11), today(), now()).unwrap();
        let a = store.snapshot(today()).anomalies[0].clone();
        assert_eq!((a.status, a.escalated, a.severity), (AnomalyStatus::Open, true, Severity::High));
        let acked = store.acknowledge(&id, now()).unwrap();
        assert_eq!((acked.status, acked.escalated), (AnomalyStatus::Acknowledged, false));
    }

    #[test]
    fn points_that_stop_firing_are_retracted_from_open_anomalies_only() {
        let store = AlertStore::in_memory();
        let both = vec![scoped("EC2", vec![finding(10, 30.0), finding(11, 30.0)]), scoped("S3", vec![finding(10, 30.0)])];
        store.apply_run("s", (d(10), d(11)), &both, run(11), today(), now()).unwrap();
        let s3 = store.snapshot(today()).anomalies.iter().find(|a| a.scope_key == "S3").unwrap().id.clone();
        store.acknowledge(&s3, now()).unwrap();
        // Revised data: EC2 day 10 no longer fires, S3 doesn't fire at all.
        store.apply_run("s", (d(10), d(11)), &[scoped("EC2", vec![finding(11, 30.0)])], run(11), today(), now()).unwrap();
        let snap = store.snapshot(today());
        let ec2 = snap.anomalies.iter().find(|a| a.scope_key == "EC2").unwrap();
        assert_eq!((ec2.first_day, ec2.points.len()), (d(11), 1));
        assert!((ec2.total_impact - 30.0).abs() < 1e-9);
        assert!(snap.anomalies.iter().any(|a| a.id == s3), "acknowledged anomalies are kept as seen");
        // Nothing fires any more: the open EC2 anomaly disappears.
        store.apply_run("s", (d(10), d(11)), &[], run(11), today(), now()).unwrap();
        let snap = store.snapshot(today());
        assert_eq!(snap.anomalies.iter().map(|a| a.scope_key.as_str()).collect::<Vec<_>>(), ["S3"]);
        // Days outside the checked range are never retracted.
        store.apply_run("s", (d(20), d(20)), &[scoped("EC2", vec![finding(20, 30.0)])], run(20), today(), now()).unwrap();
        store.apply_run("s", (d(21), d(21)), &[], run(21), today(), now()).unwrap();
        assert_eq!(store.snapshot(today()).anomalies.len(), 2);
    }

    #[test]
    fn mutes_mark_matching_anomalies_and_expire() {
        let store = AlertStore::in_memory();
        store.apply_run("s", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)]), scoped("S3", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
        store.apply_run("other", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
        store.mute("s", ScopeKind::Service, "EC2", today() + Duration::days(6), today()).unwrap();
        let snap = store.snapshot(today());
        let find = |src: &str, key: &str| snap.anomalies.iter().find(|a| a.source_id == src && a.scope_key == key).unwrap().muted;
        assert!(find("s", "EC2"));
        assert!(!find("s", "S3"));
        assert!(!find("other", "EC2"), "mutes are per source");
        let later = store.snapshot(today() + Duration::days(7));
        assert!(later.anomalies.iter().all(|a| !a.muted));
        assert!(later.mutes.is_empty());
    }

    #[test]
    fn persistence_round_trips_prunes_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alerts.json");
        {
            let store = AlertStore::open(path.clone(), today());
            store.apply_run("s", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
            store.apply_run("gone", (d(10), d(10)), &[scoped("S3", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
            store.forget_source("gone").unwrap();
        }
        assert!(!dir.path().join("alerts.json.tmp").exists());
        let reopened = AlertStore::open(path.clone(), today());
        let snap = reopened.snapshot(today());
        assert_eq!(snap.anomalies.len(), 1);
        assert_eq!(snap.runs.keys().collect::<Vec<_>>(), ["s"]);
        assert_eq!(reopened.run_state("s").unwrap().last_checked_day, Some(d(10)));

        let much_later = today() + Duration::days(RETENTION_DAYS + 30);
        assert!(AlertStore::open(path, much_later).snapshot(much_later).anomalies.is_empty());
    }

    #[test]
    fn corrupt_and_unknown_version_files_are_quarantined() {
        for content in ["{ not json", r#"{"version": 99, "anomalies": []}"#] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("alerts.json");
            std::fs::write(&path, content).unwrap();
            let store = AlertStore::open(path.clone(), today());
            assert!(store.snapshot(today()).anomalies.is_empty());
            let quarantined: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .filter(|n| n.starts_with("alerts.corrupt-") && n.ends_with(".json"))
                .collect();
            assert_eq!(quarantined.len(), 1, "{content}");
            assert!(!path.exists());
        }
    }

    #[test]
    fn failed_write_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = AlertStore::open(dir.path().join("missing-dir").join("alerts.json"), today());
        assert!(store.apply_run("s", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)])], run(10), today(), now()).is_err());
        assert!(store.snapshot(today()).anomalies.is_empty());
    }

    #[test]
    fn acknowledging_during_concurrent_runs_is_not_lost() {
        let store = std::sync::Arc::new(AlertStore::in_memory());
        store.apply_run("s", (d(10), d(10)), &[scoped("EC2", vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
        let id = store.snapshot(today()).anomalies[0].id.clone();
        let writer = {
            let store = store.clone();
            std::thread::spawn(move || {
                for i in 0..200 {
                    // An empty range: these writes must not retract anything.
                    let empty = (d(11), d(10));
                    store.apply_run("s", empty, &[scoped(&format!("svc{i}"), vec![finding(10, 30.0)])], run(10), today(), now()).unwrap();
                }
            })
        };
        store.acknowledge(&id, now()).unwrap();
        writer.join().unwrap();
        let snap = store.snapshot(today());
        assert_eq!(snap.anomalies.iter().find(|a| a.id == id).unwrap().status, AnomalyStatus::Acknowledged);
        assert_eq!(snap.anomalies.len(), 201);
    }
}
