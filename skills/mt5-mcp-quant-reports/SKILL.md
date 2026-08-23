---
name: mt5-mcp-quant-reports
description: Find, compare, annotate, export, archive, prune, and promote MT5-MCP-Quant reports and history. Use for result catalogs, best runs, baselines, tags, notes, CSV/Markdown exports, or report retention.
---

# MT5-MCP-Quant Reports and History

Resolve the right report, preserve provenance, and perform the requested catalog or lifecycle action.

## Find

- Recent/specific: `get_latest_report`, `get_report_by_id`, `list_reports`.
- Filtered: `search_reports`, `search_reports_by_date_range`, `search_reports_by_tags`, `search_reports_by_notes`, `get_reports_by_set_file`.
- Portfolio view: `get_reports_summary`, `get_best_reports`, `get_comparable_reports`, `get_history`, `get_backtest_history`.

Prefer an explicit report ID. Otherwise use report directory, then the latest matching report, and state the resolution.

## Compare and curate

- Use `compare_backtests` for side-by-side runs and `compare_baseline` for regression judgment.
- Use `promote_to_baseline` only for a verified representative report.
- Use `annotate_history` for verdict, notes, and tags.
- Use `export_deals_csv` for database-backed deals and `export_report` for CSV, JSON, or Markdown.

## Retention

- `archive_report` with `delete_after: false` is the default recoverable archive path.
- Before `archive_all_reports` or `prune_reports`, state `keep_last` and the expected scope.
- Treat deletion as a separate authorized action; report what was removed and whether an archive exists.

## Completion

Return the report IDs affected, comparison or export result, output paths, and any retention mutation performed.
