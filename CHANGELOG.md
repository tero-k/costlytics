# Changelog

All notable changes to Costlytics are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) (see "Versioning & releasing" in
the README).

## [Unreleased]

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

[Unreleased]: https://github.com/tero-k/costlytics/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tero-k/costlytics/releases/tag/v0.1.0
