---
name: mt5-mcp-quant-router
description: Route vague or underspecified MetaTrader 5 and MT5-MCP-Quant requests. Use when the user asks to use the MT5 MCP, mentions an EA without naming an operation, does not know which feature to choose, or mixes setup, backtest, optimization, report, analytics, and debugging goals.
---

# MT5-MCP-Quant Router

Turn an unclear MT5 request into safe progress without asking the user to choose among MCP tool names.

## Route by outcome

- Installation, readiness, account, symbol, or “does it work?” → apply `mt5-mcp-quant-setup-health`.
- Create, find, copy, validate, or compile MQL5 code → apply `mt5-mcp-quant-mql-development`.
- Read, edit, compare, generate, or narrow `.set` parameters → apply `mt5-mcp-quant-setfiles`.
- Test an EA, rerun it, or check stability over time → apply `mt5-mcp-quant-backtesting`.
- Find better parameter combinations or inspect an optimization → apply `mt5-mcp-quant-optimization`.
- Find, compare, tag, export, archive, or promote results → apply `mt5-mcp-quant-reports`.
- Explain profit, losses, drawdown, timing, layers, costs, or deals → apply `mt5-mcp-quant-analytics`.
- Investigate errors, crashes, logs, stuck processes, or missing output → apply `mt5-mcp-quant-debug-recovery`.

## Vague-request default

When no outcome is clear, perform this read-only orientation pass:

1. Call `healthcheck`.
2. Call `get_active_account`.
3. Call `list_experts`.
4. Call `get_latest_report`.
5. Infer the most likely route from the user’s wording and the discovered state.

If one route is clearly useful, continue with its safe first action. If materially different routes remain, ask one outcome-level question such as “EA’yı backtest etmek mi, yoksa mevcut sonucu analiz etmek mi istiyorsun?” Never ask the user to select a tool name.

## Defaults

- When the user names an EA and says “test et,” route to backtesting and use configured symbol/timeframe/date defaults after pre-flight checks.
- When the user says “iyileştir,” “en iyi ayar,” or “optimize,” route to optimization; require a valid swept `.set` file before launch.
- When the user asks “neden kaybetti?” or “sonuç nasıl?”, analyze the latest matching report unless they identify another.
- When the user reports a failure, inspect status and logs before process termination or cleanup.

## Invariants

- MT5 program and data directories are distinct on native Windows.
- Run only one Strategy Tester or optimization job per MT5 data instance.
- Use model 0 for optimization and for grid/martingale-sensitive validation.
- Treat a created report directory as progress, not completion; asynchronous tools finish only when status reports completion.
- Keep destructive maintenance, process termination, and binary updates within the user’s authorization.

## Completion

Finish when a domain workflow is selected, its safe first action has run, and the user sees the discovered context, current result, and only the input still required to continue.
