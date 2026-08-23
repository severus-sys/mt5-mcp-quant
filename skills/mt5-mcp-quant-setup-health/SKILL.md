---
name: mt5-mcp-quant-setup-health
description: Verify and troubleshoot native Windows MT5-MCP-Quant setup, terminal/data paths, account session, symbols, resources, process state, configuration, and updates. Use before the first run or when the user asks whether the MCP or MT5 is ready.
---

# MT5-MCP-Quant Setup and Health

Establish whether Codex and MT5-MCP-Quant can safely run native Windows Strategy Tester work.

## Workflow

1. Call `healthcheck` and `verify_setup`.
2. Call `get_active_account`; report login/server readiness without exposing secrets.
3. Call `check_mt5_status`, `check_mt5_process`, and `validate_mt5_config`.
4. Use `list_symbols` and `check_symbol_data_status` for the intended broker symbol and date range.
5. Use `check_system_resources` before long backtests or optimization.
6. Use `check_update` only for a requested/current-version check; use `update` only when installation is authorized.

On Windows, `diagnose_wine` and `get_wine_prefix_info` should return “not applicable”; treat that as compatibility success, not failure.

## Diagnose by symptom

- Missing EA or symbol: inspect configured data directory and the active account’s symbol suffix.
- Terminal executable missing: verify `terminal_dir` contains `terminal64.exe`, `metaeditor64.exe`, and `metatester64.exe`.
- MQL5, Tester, Bases, or config missing: correct `data_dir`; do not replace it with the program directory.
- No account: ask the user to open MT5 in the same interactive Windows session and log in.
- Existing process: report it; terminate only when it blocks an authorized job.

## Completion

Return a concise ready/not-ready verdict, the exact failing check, and the next action. Readiness requires a valid program/data split, an active account, at least one tester symbol, and no unresolved single-instance conflict.
