---
name: mt5-mcp-quant-analytics
description: Analyze MT5-MCP-Quant reports and database-backed deals across profitability, drawdown, streaks, timing, position layers, volume, costs, hold time, and efficiency. Use when the user asks why a strategy won or lost or wants targeted trading diagnostics.
---

# MT5-MCP-Quant Trade Analytics

Explain strategy behavior from the correct report and deal population.

## Resolve data

Use an explicit `report_id` when available. Otherwise resolve by report directory, then latest matching report. Deals live in SQLite; use `export_deals_csv` only when a file is requested.

## Analyze

Start with `analyze_report` for a broad view, then select only dimensions that answer the question:

- Drawdown and losses: `analyze_drawdown_events`, `analyze_top_losses`, `analyze_loss_sequences`, `analyze_streaks`.
- Direction and positions: `analyze_position_pairs`, `analyze_direction_bias`, `analyze_concurrent_peak`.
- Distribution and timing: `analyze_profit_distribution`, `analyze_monthly_pnl`, `analyze_time_performance`, `analyze_hold_time_distribution`.
- Strategy structure: `analyze_layer_performance`, `analyze_volume_vs_profit`.
- Friction and productivity: `analyze_costs`, `analyze_efficiency`.
- Raw evidence: `list_deals`, `search_deals_by_comment`, `search_deals_by_magic`.

Cross-check conclusions against trade count, date range, symbol, timeframe, and set file. Mark inference when comments or magic numbers are used to infer layers or strategy behavior.

## Completion

Return the strongest evidence, its magnitude, the affected period/deals, and a bounded recommendation. Separate observed backtest facts from proposed strategy changes and financial predictions.
