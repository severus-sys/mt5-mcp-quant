# Windows Troubleshooting

Run `verify_setup` first. It validates the executable directory, active MT5 data directory, MetaEditor, tester profiles, reports directory, and SQLite path.

## `terminal64.exe` not found

Run setup with an explicit program directory:

```powershell
.\scripts\setup.ps1 -TerminalDir 'C:\Program Files\MetaTrader 5' -Force
```

Broker-branded installations may use a different folder name. The correct directory contains `terminal64.exe`, `metaeditor64.exe`, and `metatester64.exe`.

## MT5 data directory not found

Open MT5 once, sign in, then choose **File → Open Data Folder**. Pass that path explicitly:

```powershell
.\scripts\setup.ps1 `
  -TerminalDir 'C:\Program Files\MetaTrader 5' `
  -DataDir "$env:APPDATA\MetaQuotes\Terminal\INSTANCE_ID" `
  -Force
```

Do not confuse the program directory with the data directory. A standard Windows installation separates them.

## Rust build fails with `link.exe not found`

Install Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK, then open a new terminal. Rust’s official Windows MSVC toolchain requires this linker. Alternatively use a configured `x86_64-pc-windows-gnu` toolchain.

## Application Control blocks Cargo build scripts

Corporate Windows policies may block unsigned temporary executables produced by Cargo (`os error 4551`). Build in the project’s Windows GitHub Actions job or ask the administrator to allow developer build outputs. Do not disable security policy globally.

If the policy already permits Cargo's dependency/test output directory, the project can build an optimized binary there without changing the policy:

```powershell
.\scripts\build-windows.ps1 -AppControlCompatible
```

The resulting binary is `target\debug\deps\mt5-mcp-quant-release.exe`.

## MCP server does not appear

- Use the absolute path to `mt5-mcp-quant.exe`.
- Pass `--stdio` when your client configuration requires explicit arguments.
- Restart the MCP connection after rebuilding or updating; clients keep the old process in memory.

## Backtest does not produce a report

1. Confirm MT5 is signed in and the requested symbol exists with the broker suffix.
2. Open Strategy Tester once and download history for that symbol.
3. Check `<data_dir>\MQL5\Logs` and `<data_dir>\Tester\Agent-*\logs`.
4. Use `model=0` for grid/martingale EAs.
5. Run `check_mt5_process`, `get_mt5_logs`, and `get_backtest_crash_info`.

The inactivity watchdog can stop a stuck tester and fall back to journal extraction, but full P/L analytics require the HTML/XML report.

## Symbol is missing or MT5 reports error 4302

`list_symbols` shows local Strategy Tester history; it is not the broker's full catalog. Call `ensure_market_watch_symbol` with the natural symbol such as `EURUSD`. The shared resolver may return an exact symbol or a unique broker alias such as `EURUSDm`. If it reports ambiguity, choose explicitly from the sorted candidates; no candidate is selected automatically.

A successful result verifies `selected=true` and `visible=true`. `synchronized=false` is a separate data-readiness warning, not a failed Market Watch selection.

## MQL bridge is not ready

Call `verify_setup` and inspect `mql_bridge`:

- `not_installed`: let a bridge-backed tool deploy and compile the embedded Service.
- `installed_not_running`: start `MT5McpQuantBridge` once in MT5 Navigator → Services → MT5-MCP-Quant.
- `stale`: restart the Service and confirm MT5 is responsive.
- `protocol_mismatch` or `wrong_terminal_instance`: rerun setup against the active MT5 data directory and redeploy.

The bridge is optional for the original 92 tools, so its absence does not make ordinary backtest and analytics setup fail.

## Economic calendar calls fail in Strategy Tester (error 4014)

The tester should consume static data instead of calling the live calendar API. Use `prepare_calendar_export`, poll `inspect_calendar_export`, publish with `prepare_calendar_backtest_dataset`, and include `<MT5-MCP-Quant/CalendarStaticProvider.mqh>` in the EA. Times are broker server time without a timezone suffix; empty raw-value fields mean missing and are distinct from zero. Do not run a separate Script for every export—the shared Service owns live calendar access.

## `.set` parameters or optimization flags disappear

Use `write_set_file`, `patch_set_file`, `clone_set_file`, or `set_from_optimization`. These tools write UTF-16LE with a BOM and apply the Windows read-only attribute before MT5 starts. Avoid saving optimization `.set` files as UTF-8.

## Optimization exits immediately

MT5 may leave `OptMode=-1` after an interrupted run. The pipeline resets this automatically before launch. If required, close MT5, inspect `<data_dir>\config\terminal.ini`, and rerun `validate_mt5_config`.

## Analytics says report not found

Analytics resolve data from SQLite in this order:

1. `report_id`
2. `report_dir`
3. latest report

Deals are stored in the `deals` table; `deals.csv` is only created by `export_deals_csv`.
