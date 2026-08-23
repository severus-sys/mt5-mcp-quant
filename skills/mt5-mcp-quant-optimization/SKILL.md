---
name: mt5-mcp-quant-optimization
description: Prepare, launch, monitor, and interpret native Windows MT5 parameter optimizations with MT5-MCP-Quant. Use for best-parameter searches, optimization jobs, result ranking, or follow-up set generation.
---

# MT5-MCP-Quant Optimization

Run a bounded, reproducible model-0 optimization and turn its results into usable parameter sets.

## Prepare

1. Confirm setup, account, symbol history, and system resources.
2. Read the candidate `.set` file and call `describe_sweep`.
3. Require at least one intentional sweep and report the total combinations.
4. Use model 0. Never run a second backtest or optimization on the same MT5 data instance.
5. For a smoke test, use a small explicit range or `max_passes`; do not silently turn a short request into a multi-hour search.

## Execute

1. Call `run_optimization` and capture `job_id`.
2. Use `tail_log` for launch evidence and `get_optimization_status` until terminal status.
3. Use `list_jobs` for inventory; completed status must remain consistent with the detailed status.
4. Call `get_optimization_results` and rank by the user’s metric. Apply drawdown and trade-count sanity checks rather than choosing profit alone.
5. Use `set_from_optimization` for the selected pass, optionally narrowing ranges for a second stage.
6. Backtest the selected set out of sample before calling it an improvement.

## Completion

Return the completed job ID, passes evaluated, ranking rule, winning parameters, profit/drawdown/trades, generated set path, and whether out-of-sample validation remains.
