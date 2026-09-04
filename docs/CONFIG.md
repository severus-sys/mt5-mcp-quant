# Windows Configuration Reference

## Config file

Development builds use:

```text
config\mt5-mcp-quant.yaml
```

Installed builds default to `%APPDATA%\mt5-mcp-quant\config\mt5-mcp-quant.yaml`. Override the installation root with `MT5_MCP_QUANT_HOME`.

## Required paths

```yaml
terminal_dir: 'C:\Program Files\MetaTrader 5'
data_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID'
```

`terminal_dir` contains `terminal64.exe`, `metaeditor64.exe`, and `metatester64.exe`. `data_dir` contains `MQL5`, `Tester`, `Bases`, and `config`. Do not point both keys at the program directory unless MT5 is deliberately running in portable mode.

## Full example

```yaml
terminal_dir: 'C:\Program Files\MetaTrader 5'
data_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID'
experts_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\MQL5\Experts'
indicators_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\MQL5\Indicators'
scripts_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\MQL5\Scripts'
services_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\MQL5\Services'
include_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\MQL5\Include'
terminal_common_data_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\Common'
tester_profiles_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\MQL5\Profiles\Tester'
tester_cache_dir: 'C:\Users\you\AppData\Roaming\MetaQuotes\Terminal\INSTANCE_ID\Tester'

display_mode: gui
project_dir: 'C:\dev\MyEA'
reports_dir: 'C:\dev\MyEA\reports'
opt_log_dir: 'C:\Users\you\AppData\Local\Temp\mt5-mcp-quant\logs'

backtest_symbol: EURUSD
backtest_timeframe: H1
backtest_deposit: 10000
backtest_currency: USD
backtest_model: 0
backtest_leverage: 100
backtest_timeout: 900
opt_min_agents: 1
opt_max_agents: 0
```

Run `scripts\setup.ps1` whenever the MT5 installation or active data instance changes. The script matches `%APPDATA%` instances through `origin.txt` and falls back to the most recently used instance.

The three bridge paths are optional in older YAML files and are derived automatically from `data_dir` and the MetaQuotes common-data location. `services_dir` receives the embedded Service, `include_dir` receives the static calendar provider, and `terminal_common_data_dir\Files` carries the versioned request/response protocol and datasets.
