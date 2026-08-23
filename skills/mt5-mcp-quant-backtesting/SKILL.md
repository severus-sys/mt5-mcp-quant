---
name: mt5-mcp-quant-backtesting
description: Run and monitor native Windows MT5 backtests through MT5-MCP-Quant, including full, quick, raw, background, and rolling workflows. Use when the user wants to test an EA, rerun compiled code, or evaluate stability across periods.
---

# MT5-MCP-Quant Backtesting

Produce a completed, extractable Strategy Tester report while respecting the single-instance MT5 constraint.

## Pre-flight

1. Confirm account and terminal readiness.
2. Resolve the EA with `search_experts` when needed.
3. Validate symbol history with `check_symbol_data_status`; account for broker suffixes.
4. Inspect `check_mt5_process`. Use `kill_existing` only within the user’s authorization.
5. Prefer model 0 for correctness-sensitive and grid/martingale strategies.

## Choose the run mode

- Source changed or compilation must be verified: `run_backtest` (blocking full pipeline).
- Existing `.ex5`, normal analysis wanted: `run_backtest_quick` (asynchronous).
- Existing `.ex5`, raw extraction only: `run_backtest_only` (asynchronous).
- Explicit background job with custom skip flags: `launch_backtest`.
- Stability across consecutive weeks: `run_rolling_backtest` (asynchronous).

For every asynchronous response, capture `report_dir` and poll `get_backtest_status`. Continue while `is_complete` is false. A report directory or `report_found: true` alone is not completion.

Use `get_tester_log` during or after a run when trade activity or report flushing is unclear.

## Completion

Finish only when status is completed or failed, MT5 has reached the expected shutdown state, and the report/metrics exist. On success, summarize report ID, duration, trades, net profit, drawdown, and the next relevant analysis.
