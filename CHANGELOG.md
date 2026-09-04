# Changelog

All notable changes to MT5-MCP-Quant are documented here.

## [Unreleased]

### Fixed

- Prevented legitimate symbols ending in uppercase `C` from being misclassified as cent-account aliases.
- Aligned Rust and MQL terminal-instance path casing, verified deployed Service source contents, and removed queued requests after client timeouts.
- Made prepared calendar jobs retry bridge installation, recover completed responses after MCP restarts, and remain reusable for multiple tester datasets.
- Made calendar dataset publication transactional with provider deployment and persistent job metadata.

## [1.35.0] - 2026-09-04

### Added

- Added `ensure_market_watch_symbol` with broker-catalog discovery, deterministic alias resolution, ambiguity refusal, and verified Market Watch selection.
- Added an embedded, chart-independent `MT5McpQuantBridge` MQL5 Service with an allowlisted, versioned `FILE_COMMON` protocol and optional bridge health reporting.
- Added persistent asynchronous economic-calendar export jobs through `prepare_calendar_export` and `inspect_calendar_export`.
- Added checksummed CSV v1 Strategy Tester datasets through `prepare_calendar_backtest_dataset` and `CMt5MqCalendarStaticProvider`.
- Added the grouped `mt5-mcp-quant-calendar-data` Agent Skill, bringing the portable workflow-skill count to 10.

### Changed

- Centralized symbol resolution across backtest, rolling, optimization, data-status, and cache workflows. Ambiguous broker affixes are no longer resolved by choosing the first candidate.
- Expanded the MCP surface from 92 to exactly 96 tools without removing or renaming any existing tool.
- Clarified that `list_symbols` represents local Strategy Tester history, while Market Watch uses the live broker catalog.
- Calendar export now uses the shared Service rather than requiring a Script to be launched for every export.
- Release publishing targets GitHub Releases with both a portable archive and a native Windows MCPB, then publishes the verified MCPB to the MCP Registry. crates.io publishing is intentionally excluded.

### Fixed

- Corrected tester history discovery to prefer `Tester/bases` and fall back to `Bases` only when needed.
- Prevented per-symbol cache cleanup from deleting files when a symbol alias is ambiguous.
- Added structured requested/resolved symbol context and actionable no-history/no-match failures.
- Standardized FILE_COMMON protocol timestamps on UTC and reject implausibly future heartbeats, preventing false-ready health and immediately expired requests on non-UTC Windows systems.

[1.35.0]: https://github.com/severus-sys/mt5-mcp-quant/releases/tag/v1.35.0
