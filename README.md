# MT5-MCP-Quant

**Native Windows MCP server for MetaTrader 5 strategy development.** Compile, backtest, analyze, optimize, debug, manage Market Watch, and build static economic-calendar datasets through 96 MCP tools—without Wine, WSL, or runtime GUI automation.

The repository, Rust crate, executable, MCP registration examples, and Agent Skill identifiers all use the `mt5-mcp-quant` / `mt5_mcp_quant` project identity.

```
You: "Backtest MyEA Jan-Mar, what caused the February drawdown?"

Claude: [compile → clean → backtest → analyze 1,847 deals]
        → Feb 14: BUY grid at L6, locking lot 1.75× base
        → Cutloss fired 17 points later
        → Recommendation: cap locking multiplier to ≤1.2×
```

## Why MT5-MCP-Quant

**Focus:** Backtest organization, reporting, and analytics — capabilities MT5 itself doesn't provide.

| | MT5-MCP-Quant | Others |
|---|---|---|
| **Platform** | Native Windows | Windows integrations |
| **Backtest pipeline** | ✅ Full (compile → run → analyze) | ✅ Via MT5 Python package |
| **Deal-level analytics** | ✅ 19 dimensions, DB-backed | ❌ |
| **Report organization** | ✅ SQLite (reports + deals) + search + history | ❌ |
| **MQL5 compilation** | ✅ MetaEditor CLI | ⚠️ Via GUI or terminal |
| **Optimization** | ✅ Background + results parsing + .set generation | ⚠️ Terminal only, no parsing |
| **Crash debugging** | ✅ Native process, log, and resource diagnostics | ❌ |

Unlike integrations centered on the MetaTrader5 Python trading API, MT5-MCP-Quant focuses on **Strategy Tester automation, report organization, deal-level insight, and optimization workflows**.

## Quick Start

### Do I need to manage an `.exe`?

**No, not during normal use.** MT5-MCP-Quant is a native Windows MCP server, so the MCP client must start a Windows process in the background. That process is technically an `.exe`, but it is a one-time installation detail:

- Do not double-click the binary. It speaks JSON-RPC over stdio and is meant to be started by an MCP client.
- An AI coding agent can download or build it, save its path in the client configuration, and verify the connection once.
- After registration, use natural-language prompts. Codex, Claude Code, OpenCode, or Hermes starts and stops MT5-MCP-Quant automatically.
- `cargo run` does not remove the executable; Rust still builds one internally and makes every MCP startup slower. A registered release build is the recommended runtime.

In short: **the runtime needs an executable, but the user should not have to manage it.**

### Requirements

- Windows 10/11 x64 and native MetaTrader 5, opened once under the same user account
- A logged-in MT5 demo or live account in the same interactive Windows session
- For a pre-built release: no Rust or Visual Studio installation is required
- For source builds only: [Rust stable](https://rust-lang.org/tools/install/) and Visual Studio’s [Desktop development with C++](https://learn.microsoft.com/en-us/visualstudio/install/workload-component-id-vs-build-tools?view=visualstudio) workload

### Agent-managed installation

Give this prompt to Codex, Claude Code, OpenCode, or Hermes from the repository directory:

> Install and configure MT5-MCP-Quant on this Windows computer. Prefer the official pre-built Windows x64 release; build from source only when needed. Detect MT5 with `scripts/setup.ps1`, register the server as `mt5_mcp_quant` over stdio, install the Agent Skills for this client, and verify that `healthcheck` works and exactly 96 tools are available. Do not ask me to launch the binary manually.

The agent should complete the whole one-time setup:

1. Download the Windows x64 release, or build the Rust source when a release is unavailable.
2. Run `powershell -ExecutionPolicy Bypass -File scripts\setup.ps1` to detect the MT5 program and data directories.
3. Register the runtime command and `--stdio` in the client's MCP configuration.
4. Install the router and workflow skills with `scripts\install-agent-skills.ps1`.
5. Ask for one client restart, then confirm MT5 readiness and the exact 96-tool inventory.
6. For Market Watch or calendar work, compile the embedded `MT5McpQuantBridge` Service and ask the user to start it once from MT5 Navigator → Services → MT5-MCP-Quant.

The stored binary path is an internal runtime setting after step 3. Normal users should not need to remember, open, or pass that path in prompts.

### Install Agent Skills

Install the router and nine workflow skills for Codex, Claude Code, OpenCode, and Hermes:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install-agent-skills.ps1 -Client all
```

Users can then write a vague prompt such as “MT5 MCP'yi kullan ve EA'me bak.” The router performs safe discovery and selects the appropriate workflow without requiring a tool name. See [Agent Skills](docs/AGENT_SKILLS.md).

The router starts with read-only orientation checks: MCP health, active account, available EAs, and the latest report. It then chooses setup, MQL development, set-file management, backtesting, optimization, calendar data, reporting, analytics, or recovery. When two materially different goals remain possible, it asks one outcome-level question instead of asking the user to choose an MCP tool.

### First Backtest

```
Run a backtest on MyEA from 2025.01.01 to 2025.03.31
```

The AI runs the full pipeline: compile → clean cache → backtest → extract → analyze.

## MCP Tools (96)

### Core workflow

| Tool | Description |
|------|-----|
| `run_backtest` | Full pipeline: compile → clean → backtest → extract → analyze |
| `run_backtest_quick` | Async quick backtest using pre-compiled EA; poll status |
| `run_backtest_only` | Async raw backtest without compile/analysis; poll status |
| `launch_backtest` | Fire-and-forget: launch MT5 backtest, poll for completion |
| `get_backtest_status` | Poll running backtest status (MT5 running, report found, elapsed time) |
| `run_optimization` | Genetic optimization (background, returns immediately) |
| `get_optimization_results` | Parse optimization results after MT5 finishes |
| `list_jobs` | List all optimization jobs with status |
| `analyze_report` | Read `analysis.json` from any report directory |
| `compare_baseline` | Compare report vs baseline, return winner/loser verdict |
| `compile_ea` | Compile MQL5 EA via MetaEditor |
| `list_experts` | List all EAs in MQL5/Experts directory |
| `list_indicators` | List all indicators in MQL5/Indicators directory |
| `list_scripts` | List all scripts in MQL5/Scripts directory |
| `healthcheck` | Quick server health check |
| `list_symbols` | List symbols with local Strategy Tester history |

### Market Watch and economic calendar

| Tool | Description |
|------|-----|
| `ensure_market_watch_symbol` | Resolve broker suffix/prefix aliases without guessing, select the exact symbol, and verify Market Watch visibility |
| `prepare_calendar_export` | Create an idempotent asynchronous export job against the active broker's live MT5 calendar |
| `inspect_calendar_export` | Poll job state, progress, row count, coverage, validation, and machine-readable errors |
| `prepare_calendar_backtest_dataset` | Publish a validated CSV v1 dataset and checksum manifest for Strategy Tester EAs |

The embedded `MT5McpQuantBridge` MQL5 Service is optional for the original 92 tools and required only for broker-catalog/Market Watch and live-calendar operations. Start it once from MT5 Navigator; unlike the earlier proposal, calendar export does not require running a new Script for every export.

### Granular Analytics (individual analysis)

| Tool | Description |
|------|-----|
| `analyze_monthly_pnl` | Monthly P/L breakdown only |
| `analyze_drawdown_events` | Drawdown events and causes only |
| `analyze_top_losses` | Worst losing deals only |
| `analyze_loss_sequences` | Consecutive loss patterns only |
| `analyze_position_pairs` | Position hold time and P/L pairs |
| `analyze_direction_bias` | Buy vs Sell performance |
| `analyze_streaks` | Win/loss streak analysis |
| `analyze_concurrent_peak` | Peak simultaneous positions |

Use these for targeted analysis, or `analyze_report` to run all at once.

### Deal-Level Analytics (New)

| Tool | Description |
|------|-----|
| `list_deals` | List individual deals with filters (type, profit range, volume, dates) |
| `search_deals_by_comment` | Full-text search in deal comments (e.g., "Layer #3") |
| `search_deals_by_magic` | Filter deals by EA magic number |
| `analyze_profit_distribution` | Profit histogram: small/medium/large wins and losses |
| `analyze_time_performance` | Performance by hour of day and day of week |
| `analyze_hold_time_distribution` | Hold time buckets + correlation with profit |
| `analyze_layer_performance` | Grid/martingale layer analysis from comments |
| `analyze_volume_vs_profit` | Volume correlation + performance by lot size |
| `analyze_costs` | Commission and swap impact on profitability |
| `analyze_efficiency` | Profit per hour/day, annualized return, trade frequency |

### Monitoring

| Tool | Description |
|------|-----|
| `verify_setup` | Check native MT5 program/data paths and EA/set file inventory |
| `get_optimization_status` | Check live state of a background optimization job |
| `list_jobs` | All optimization jobs with compact status in one call |

### Reports & logs

| Tool | Description |
|------|-----|
| `list_reports` | Compact table of all runs with key metrics — no full analysis needed |
| `get_latest_report` | Get most recent report with optional equity chart |
| `search_reports` | Find reports by EA, symbol, date range, or profit criteria |
| `get_report_by_id` | Get specific report by ID with equity chart |
| `get_reports_summary` | Aggregate stats: counts, averages, pass rates |
| `get_best_reports` | Top N reports sorted by any metric (profit factor, drawdown, etc.) |
| `search_reports_by_tags` | Find reports by tags |
| `search_reports_by_date_range` | Query by backtest date range |
| `search_reports_by_notes` | Full-text search in report notes |
| `get_reports_by_set_file` | Find all reports using a specific .set file |
| `get_comparable_reports` | Find comparable reports (same EA/symbol/timeframe) |
| `tail_log` | Read last N lines of any log; `filter=errors` to see only failures |
| `prune_reports` | Delete old report directories, keep last N (skips `_opt` dirs) |
| `promote_to_baseline` | Write a history entry or report to `baseline.json` for compare_baseline |

### History & baseline

| Tool | Description |
|------|-----|
| `archive_report` | Convert one report dir → compact JSON entry in `backtest_history.json`, optionally delete source |
| `archive_all_reports` | Bulk-archive all report dirs then optionally delete them; keeps N newest safe |
| `get_history` | Query history with filters (EA, symbol, verdict, profit, DD) and sort options |
| `annotate_history` | Attach verdict / notes / tags to any history entry |

### Cache management

| Tool | Description |
|------|-----|
| `cache_status` | MT5 tester cache size breakdown by symbol — check before cleaning |
| `clean_cache` | Delete tester cache files; supports per-symbol and `dry_run` |

### Pre-flight & Validation

| Tool | Description |
|------|-----|
| `get_active_account` | Get current MT5 account session (login, server, available symbols) |
| `check_symbol_data_status` | Validate symbol has sufficient history data for date range |
| `check_mt5_status` | Check if MT5 terminal is installed and ready |
| `validate_ea_syntax` | Pre-compile syntax check without running full compilation |

### Debugging & Diagnostics (New)

| Tool | Description |
|------|-----|
| `diagnose_wine` | Legacy compatibility tool; reports native-Windows runtime status on Windows |
| `get_mt5_logs` | Get MT5 terminal, tester, or MetaEditor logs with filtering |
| `search_mt5_errors` | Search logs for error patterns (crash, exception, access violation) |
| `check_mt5_process` | Check if MT5 processes are running, get PID, CPU, memory usage |
| `kill_mt5_process` | Stop stuck native MT5 and tester processes |
| `check_system_resources` | Check disk space, memory, CPU availability |
| `validate_mt5_config` | Validate terminal.ini and tester configuration files |
| `get_wine_prefix_info` | Legacy compatibility tool; returns MT5 data-folder information on Windows |
| `get_backtest_crash_info` | Investigate backtest failures: incomplete markers, missing deals.csv, errors |
| `check_update` | Check if a newer version of MT5-MCP-Quant is available |
| `update` | Update MT5-MCP-Quant to latest release |

### Project Management

| Tool | Description |
|------|-----|
| `init_project` | Scaffold new MQL5 project with templates (scalper/swing/grid/basic) |
| `create_set_template` | Generate .set parameter file from EA input variables |
| `export_report` | Export backtest report to CSV, JSON, or Markdown |

### History & Comparison

| Tool | Description |
|------|-----|
| `get_backtest_history` | List all backtests for EA/symbol with summary metrics |
| `compare_backtests` | Compare 2+ backtest results side-by-side with analysis |

### .set file — read / write

| Tool | Description |
|------|-----|
| `list_set_files` | All .set files in tester profiles dir with sweep stats and combination counts |
| `read_set_file` | Parse UTF-16LE `.set` file → structured JSON params |
| `write_set_file` | Write params → UTF-16LE `.set` with the Windows read-only attribute |
| `patch_set_file` | Update specific params in-place, return diff — replaces read→edit→write |
| `clone_set_file` | Copy `.set` to new path with optional overrides in one call |

### .set file — analysis & generation

| Tool | Description |
|------|-----|
| `describe_sweep` | Swept params, value counts, and total optimization combinations |
| `diff_set_files` | Side-by-side diff of two `.set` files — only changed params returned |
| `set_from_optimization` | Generate a clean backtest `.set` from `get_optimization_results` params; optionally narrow sweep |

### Search & Discovery

| Tool | Description |
|------|-----|
| `search_experts` | Search EAs by name pattern across all directories |
| `search_indicators` | Search indicators by name pattern |
| `search_scripts` | Search scripts by name pattern |
| `copy_indicator_to_project` | Copy indicator to project directory |
| `copy_script_to_project` | Copy script to project directory |

Full schema: [docs/MCP_TOOLS.md](docs/MCP_TOOLS.md)

## Troubleshooting

Run `verify_setup` from your LLM first — it checks all paths and returns actionable hints.

For crashes or unexplained failures during backtest/compile/optimization:
- `check_mt5_status` — Check native terminal, MetaEditor, tester, and data paths
- `search_mt5_errors` — Find crash causes in logs
- `check_mt5_process` + `kill_mt5_process` — Detect and kill stuck processes
- `get_backtest_crash_info` — Investigate failed backtest reports

**[Full Troubleshooting Guide →](docs/TROUBLESHOOTING.md)**

---

## Acknowledgements

MT5-MCP-Quant stands on the shoulders of exceptional open-source projects:

- **[Rust](https://www.rust-lang.org/)** — The language that makes zero-cost abstractions, memory safety, and fearless concurrency practical
- **[Tokio](https://tokio.rs/)** — The async runtime powering all concurrent operations
- **Windows** — Native process execution and filesystem integration for MetaTrader 5
- **[MetaTrader 5](https://www.metatrader5.com/)** — MetaQuotes' trading platform (trademark of MetaQuotes Software Corp.)
- **[rusqlite](https://github.com/rusqlite/rusqlite)** — Ergonomic SQLite bindings for Rust
- **[serde](https://serde.rs/)** — The serialization framework making config and report handling painless
- **[scraper](https://github.com/causal-agent/scraper)** — HTML parsing for MT5 report extraction
- **[tempfile](https://github.com/Stebalien/temp-file)** — Secure temporary file handling

Special thanks to the Model Context Protocol (MCP) team at Anthropic for defining the standard that makes AI-powered development workflows possible.

## Disclaimer

**Not Financial Advice.** MT5-MCP-Quant is a development and analysis tool for algorithmic trading strategies. It does not provide investment advice, trading recommendations, or guarantee profitability. All backtest results are historical simulations and do not guarantee future performance.

**Use at Your Own Risk.** Trading financial instruments carries substantial risk of loss. The authors and contributors of MT5-MCP-Quant accept no liability for:
- Trading losses incurred using strategies developed or tested with this tool
- Data loss or corruption from backtest operations
- Bugs, errors, or incorrect analysis results
- System crashes, native MT5 process failures, or broker/runtime incompatibilities

**Software Warranty.** This software is provided "as-is" without warranty of any kind, express or implied, including but not limited to warranties of merchantability, fitness for a particular purpose, or non-infringement. See LICENSE for full terms.

**Third-Party Software.** MT5-MCP-Quant interacts with MetaTrader 5 and other third-party software. Users are responsible for complying with all applicable licenses and terms of service. MetaTrader is a trademark of MetaQuotes Software Corp. MT5-MCP-Quant is not affiliated with, endorsed by, or sponsored by MetaQuotes.

**Regulatory Compliance.** Users are responsible for ensuring their trading activities comply with applicable financial regulations in their jurisdiction. Automated trading may be restricted or require licensing in some regions.

## License

[MIT License](LICENSE)

---
