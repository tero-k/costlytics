# Changelog

All notable changes to Costlytics are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) (see "Versioning & releasing" in
the README).

## [Unreleased]

## [0.2.0] - 2026-10-18

### Added
- Anomalies: watches daily spend per service and per account and flags
  changes that are both unusual for that scope (a weekday-aware baseline of
  recent days) and big enough to matter (minimum change in dollars and
  percent, optional monthly impact), so low-cost noise such as $0.10 → $1
  never alerts.
  - Only usage charges are watched, so tax, Support, Savings Plan and
    Reserved Instance fees, credits and refunds don't alert. A charge that
    recurs on the same day each month is not reported again.
  - New spend, and runaway days on services billed only now and then, are
    flagged too. Decreases are opt-in.
  - An Anomalies page lists them by severity with a sparkline, expected vs.
    actual and total impact. Acknowledge, mute (7, 30 or 90 days, per
    source) or open the matching detail page. An acknowledged anomaly
    reopens tagged "Escalated" if its severity rises.
  - A sidebar badge counts open anomalies. Checks run while the app is open,
    when new billing data arrives (re-checked every 6 hours), after the rules
    change, and on demand with **Check now**, and they respect cost guard.
  - Settings → Anomaly rules, with overrides for individual services or
    accounts. Inbox state is kept in `alerts.json` next to `settings.toml`.

### Changed
- The macOS release workflow accepts Developer ID certificates exported with
  current AES-256 encryption: it re-packs them into the format macOS
  `security import` accepts before signing.
- The macOS release workflow's build job runs with a read-only GitHub token;
  only the job that uploads to the release can write.

## [0.1.0] - 2026-09-26

First public release.

### Added
- Tauri v2 desktop app for Windows and macOS, with S3 secrets stored in the
  OS keychain.
- Local and S3 data sources in FOCUS 1.0, FOCUS 1.2 and AWS CUR 2.0 (Parquet)
  formats, managed from the Settings page.
- Overview, Cost Explorer, Cost Changes, and Service, Account, Resource and
  Tags drill-down pages, with shared source, date, metric and filter
  controls, CSV export, and views that can be shared by URL.
- Cost guard, which estimates S3 read costs before a page load and warns or
  asks for confirmation above configurable limits.
- App version shown at the bottom of the sidebar, with the git build id in
  its tooltip.
- `run-local.ps1` for running an optimized build in the browser against the
  sample data.
- Dev/test HTTP harness (`costlytics-api`) and a Playwright end-to-end suite.
- README with a usage guide and screenshots, and the MIT license.

[Unreleased]: https://github.com/tero-k/costlytics/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/tero-k/costlytics/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tero-k/costlytics/releases/tag/v0.1.0
