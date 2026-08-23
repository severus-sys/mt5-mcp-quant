---
name: mt5-mcp-quant-debug-recovery
description: Diagnose and recover native Windows MT5-MCP-Quant failures, crashes, missing reports, log errors, stuck Strategy Tester processes, cache issues, and incomplete jobs. Use when a compile, backtest, optimization, or MCP operation fails or hangs.
---

# MT5-MCP-Quant Debug and Recovery

Find the failure boundary before changing state.

## Read-only diagnosis

1. Call `healthcheck`, `check_mt5_status`, and `check_mt5_process`.
2. Inspect `get_backtest_status` or `get_optimization_status` for the active job.
3. Use `get_tester_log`, `get_mt5_logs`, and `tail_log` for direct evidence.
4. Use `search_mt5_errors`; successful “0 errors” summaries are not failures.
5. Use `get_backtest_crash_info`, `validate_mt5_config`, and `check_system_resources`.
6. Check symbol history and broker suffix when a run creates no trades or report.

## Recovery

- If the configured MT5 process is demonstrably stuck and termination is authorized, call `kill_mt5_process` for the exact PID or configured instance.
- Inspect `cache_status` before `clean_cache`; use dry-run first when diagnosis does not require deletion.
- Reset or rerun only the failed job. Preserve report directories and logs until evidence is captured.
- Use `update` only when a verified version issue justifies it and binary replacement is authorized.

## Known MT5 boundaries

- Program and data directories are distinct.
- MT5 must run in the same interactive Windows user session as the MCP server.
- One data instance supports one Strategy Tester/optimization job at a time.
- SpreadsheetML `.htm.xml` may be the real report even when HTML is absent.
- After optimization, the next backtest requires `OptMode=0`; the pipeline cleanup owns this reset.

## Completion

Return the failing stage, direct log/status evidence, root cause or remaining hypothesis, recovery action, and proof that the original symptom no longer reproduces.
