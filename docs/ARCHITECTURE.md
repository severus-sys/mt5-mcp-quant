# Architecture

## Design Principles

**Deal-level over aggregate.** MT5's HTML report gives you: profit, profit factor, max DD%, trade count. MT5-MCP-Quant extracts every individual deal — entry price, exit price, P/L, comment string — to reconstruct what happened during each loss event. Result: `analysis.json`, AI-readable and diffable between runs.

**Pipeline idempotency.** MT5 caches aggressively (`.ex5` binaries, `.set` files, `terminal.ini` flags). The pipeline invalidates all cache before every run to prevent stale results.

**Background isolation.** Genetic optimizations run for hours. Native Windows launches `terminal64.exe` as an independent child and persists job metadata under the OS temporary directory, so MCP clients can reconnect and poll it.

**Program/data separation.** Native MT5 keeps executables under Program Files and instance data under `%APPDATA%\MetaQuotes\Terminal`. `Config` models these as `terminal_dir` and `data_dir`; all MQL5, tester, report, and account paths derive from `data_dir`.

**Allowlisted native bridge.** Market Watch and live economic-calendar access run inside one embedded MQL5 Service. Rust and the Service exchange versioned, terminal-instance-bound files under `FILE_COMMON`; the allowlist contains only symbol catalog, exact selection, and calendar export operations.

---

## Component Map

```
MT5-MCP-Quant/
├── src/
│   ├── main.rs                 # MCP server entry (stdio transport)
│   ├── mcp_server.rs           # MCP protocol handling
│   ├── models/                 # Data structures
│   │   ├── config.rs           # Configuration
│   │   ├── deals.rs            # Deal, PositionPair, DrawdownEvent, etc.
│   │   ├── metrics.rs          # Metrics parsing from HTML/XML
│   │   └── report.rs           # Report, PipelineMetadata, etc.
│   ├── analytics/              # Report extraction & analysis (migrated from Python)
│   │   ├── extract.rs          # HTML/XML report parser → metrics.json (deals → DB)
│   │   └── analyze.rs          # Deal-level analysis engine → analysis.json
│   ├── compile/                # MQL5 compilation
│   │   └── mql_compiler.rs     # Expert/Indicator/Script/Service compiler + Include deployer
│   ├── bridge.rs               # FILE_COMMON protocol, heartbeat, identity and deployment
│   ├── pipeline/               # Backtest orchestration
│   │   ├── backtest.rs         # 5-stage pipeline (COMPILE→CLEAN→BACKTEST→EXTRACT→ANALYZE)
│   │   └── stages.rs           # Pipeline stage definitions
│   ├── storage/                # SQLite persistence
│   │   └── database.rs         # ReportDb: reports table + deals table
│   └── tools/                  # MCP tool definitions
│       ├── definitions/        # Tool schemas (12 domain modules, 96 tools)
│       │   ├── mod.rs
│       │   ├── analytics.rs      # 19 analysis tools (DB-backed)
│       │   ├── backtest.rs       # 7 backtest tools
│       │   ├── baseline.rs       # 1 baseline tool
│       │   ├── experts.rs        # 9 EA/indicator/script tools
│       │   ├── market_watch.rs   # 1 broker catalog / Market Watch tool
│       │   ├── calendar.rs       # 3 calendar export / dataset tools
│       │   ├── optimization.rs   # 4 optimization tools
│       │   ├── reports.rs        # 20 report management tools
│       │   ├── setfiles.rs       # 8 .set file tools
│       │   └── system.rs         # 6 system tools
│       └── handlers/             # Tool dispatch (11 domain modules)
│           ├── mod.rs
│           ├── analysis.rs
│           ├── backtest.rs
│           ├── experts.rs
│           ├── market_watch.rs
│           ├── calendar.rs
│           ├── optimization.rs
│           ├── reports.rs
│           ├── setfiles.rs
│           └── system.rs
│
├── mql/
│   ├── MT5McpQuantBridge.mq5     # Chart-independent MQL5 Service
│   └── CalendarStaticProvider.mqh # Checksummed Strategy Tester provider
├── scripts/
│   ├── setup.ps1               # Auto-detect native MT5 program/data paths
│   ├── setup.sh                # Legacy macOS/Linux setup
│   ├── platform_detect.sh      # Legacy Wine/headless detection
│   ├── build-rust.sh           # Rust build script
│   └── optimize.sh             # Legacy optimization driver
│
├── config/
│   ├── mt5-mcp-quant.example.yaml  # Template config
│   └── mt5-mcp-quant.yaml          # Live config (gitignored)
│
└── docs/
    ├── ARCHITECTURE.md         # This file
    ├── MCP_TOOLS.md            # Full tool spec
    └── REMOTE_AGENTS.md        # Optional remote tester-agent setup
```

---

## Pipeline Stages

### Stage 1: COMPILE

```rust
// src/compile/mql_compiler.rs
let compiler = MqlCompiler::new(config);
let result = compiler.compile("src/experts/MyEA.mq5")?;
```

Invokes native `metaeditor64.exe` with `/compile:` and `/log`. Sources are staged under `%TEMP%\mt5-mcp-quant\compile`, then the resulting `.ex5` and source tree are synchronized to the active data instance’s `MQL5\Experts` directory.

**Why not skip this?** MT5 caches the `.ex5` binary by filename. If you edit your EA and re-run without recompiling, MT5 runs the old binary silently. Always compile.

---

### Stage 2: CLEAN

```powershell
Remove-Item "$DataDir\Tester\**\*.tst"
Remove-Item "$DataDir\MQL5\Profiles\Tester\$Expert.set"
```

Clears:
- Tester cache (`.tst` files): compiled test results MT5 reuses to skip re-running ticks
- Cached `.set` file: MT5 writes the current parameter values here after each run; if stale, next run picks up wrong params

**The `.set` encoding trap:** MT5 reads parameter files as UTF-16LE with BOM. It writes them back as UTF-16LE. If you provide a UTF-8 `.set` file for optimization, MT5 reads the parameters correctly (it tries multiple encodings) but when it writes the optimization variants, it uses UTF-16LE and **strips the `||Y` optimization flags**. Every subsequent pass uses the base value. Your 500-combination optimization runs 500 identical backtests.

Solution: always write `.set` files as UTF-16LE with BOM, mark them read-only before MT5 starts.

```python
# Python: write UTF-16LE with BOM
content = "\n".join(lines)
with open(set_path, 'w', encoding='utf-16-le') as f:
    f.write('\ufeff')  # BOM
    f.write(content)
path.set_readonly(True)  # Windows read-only attribute
```

---

### Stage 3: BACKTEST

```powershell
& 'C:\Program Files\MetaTrader 5\terminal64.exe' "/config:$env:TEMP\mt5-mcp-quant\backtest_config.ini"
```

`backtest.ini` contains:
```ini
[Tester]
Expert=MyEA
Symbol=XAUUSD
Period=M5
Deposit=10000
Currency=USD
Leverage=1:500
Model=0
FromDate=2025.01.01
ToDate=2025.06.30
Report=C:\report
Optimization=0
```

MT5 starts the native Strategy Tester, writes the report into `<data_dir>\reports`, and exits when `ShutdownTerminal=1`. A process watchdog handles builds that remain open or stop producing tester-log activity.

---

### Stage 4: EXTRACT + STORE

Single HTML/XML parse pass. Deals go directly into the SQLite database; the raw report file is deleted afterwards.

```rust
// src/analytics/extract.rs
let extractor = ReportExtractor::new();
let result = extractor.extract(&report_path, &output_dir)?;
// → metrics.json  (aggregate summary — written to report_dir)
// HTML report deleted after extraction

// src/storage/database.rs
db.insert_deals(&report_id, &result.deals)?;
// → deals table in SQLite (all deals, keyed by report_id)
```

On-demand CSV export is available via the `export_deals_csv` tool:
```
export_deals_csv(report_id: "20260422_051041_DPS21_XAUUSDc_M5_1")
// → report_dir/deals.csv  (written only when explicitly requested)
```

**Why single-pass?** MT5 HTML reports are large (1-5MB for 14-month tests). Each regex pass over the file takes ~200ms. The old pipeline ran 5 separate grep/regex passes. The Rust implementation uses a single-pass parser: 5× faster and no partial-read inconsistencies.

**Format detection:**
```rust
// MT5 Build 48+ saves SpreadsheetML XML, not HTML
let ext = Path::new(&path).extension()
    .and_then(|e| e.to_str())
    .unwrap_or("");

if ext == "xml" || path.ends_with(".htm.xml") {
    // Parse as SpreadsheetML XML
    let doc = roxmltree::Document::parse(&text)?;
} else {
    // Parse as HTML with regex
}
```

**Deal columns (stored in DB):**
```
time | deal | symbol | deal_type | entry | volume | price | order_id
commission | swap | profit | balance | comment | magic
```

The `comment` column is the key to grid analytics. The EA writes `"Layer #3"`, `"Locking Total"`, `"Zombie Exit"` etc. Pattern matching on comments reconstructs which position was at which layer.

---

### Stage 5: ANALYZE

```rust
// src/analytics/analyze.rs
let analyzer = DealAnalyzer::new();
let result = analyzer.analyze(&deals, &metrics, strategy, deep)?;
// → analysis.json
```

All functions operate on the parsed deal data — no MT5 or Wine required.

**Strategy profiles** (defined in `analyze.rs`):
- `grid` — Layer depth tracking, locking/cutloss/zombie keywords
- `scalper` — TP/SL/manual/trailing exit classification
- `trend` — TP/SL/trailing/breakeven/partial exits
- `hedge` — TP/SL/net_close/partial, magic+direction grouping
- `generic` — Simple profit-based TP/SL classification

#### Strategy profiles

The analysis engine is driven by a `PROFILES` dict. Each profile controls:

| Field | Type | Controls |
|-------|------|----------|
| `depth_re` | regex or `None` | Whether/how to extract depth from comments |
| `exit_keywords` | `{reason: [kw]}` | Comment patterns for exit classification |
| `dd_cause_keywords` | `{cause: [kw]}` | Comment patterns for DD cause classification |
| `cycle_group_by` | `'magic'` or `'magic+direction'` | How deals are grouped into cycles |
| `cycle_gap_min` | int | Minutes between opens that mark a new cycle |

Built-in profiles:

| Profile | `depth_re` | `cycle_group_by` | `cycle_gap_min` | Exit keywords |
|---------|-----------|-----------------|----------------|---------------|
| `generic` | — | `magic` | 60 | profit-sign only (tp/sl) |
| `grid` | `Layer #N` | `magic+direction` | 60 | locking, cutloss, zombie, timeout |
| `scalper` | — | `magic` | 10 | tp, sl, manual, trailing |
| `trend` | — | `magic` | 240 | breakeven, trailing, partial, tp, sl |
| `hedge` | — | `magic+direction` | 120 | tp, sl, net_close, partial |


#### Analytics functions

**Core (always run, strategy-agnostic):**

| Function | What it computes |
|----------|-----------------|
| `monthly_pnl` | P/L, trade count, green flag per calendar month |
| `reconstruct_dd_events` | Balance curve → local minima; cause from profile keywords |
| `top_losses` | Worst individual closing deals by P/L |
| `loss_sequences` | Consecutive losing closed deals (runs of length ≥ 2) |
| `position_pairs` | Match in/out by order ticket → hold time, depth at close |
| `direction_bias` | Buy vs sell win rate, total P/L, average trade |
| `streak_analysis` | Max consecutive win/loss streaks; current streak |
| `session_breakdown` | Asian (00–08h) / London (08–13h) / London-NY (13–17h) / New York (17–22h) |
| `weekday_pnl` | Mon–Sun P/L and win rate |
| `concurrent_peak` | Peak simultaneous open positions |

**Strategy-driven (output varies by profile):**

| Function | Generic | Grid | Scalper/Trend/Hedge |
|----------|---------|------|---------------------|
| `depth_histogram` | `{}` (empty) | L1–L8+ counts | `{}` (no `depth_re`) |
| `cycle_stats` | magic, 60-min gap | magic+direction, 60-min gap | per-profile config |
| `exit_reason_breakdown` | tp / sl | locking / cutloss / zombie / timeout | profile-specific |

**Deep analytics (`--deep` flag):**

| Function | What it computes |
|----------|-----------------|
| `hourly_pnl` | Hour-by-hour (0–23) P/L and win rate |
| `volume_profile` | P/L breakdown by lot size tier |

**DD event reconstruction:**
1. Walk deals chronologically, track running balance
2. At each local minimum (DD > 1%), record timestamp, depth (%), recovery date
3. Classify `cause` using `profile['dd_cause_keywords']`; returns `"unknown"` for generic/unmatched

**Cycle statistics:**
Deals are grouped by `cycle_group_by` key. A gap greater than `cycle_gap_min` between consecutive opens marks a new cycle boundary. Win rate is computed per cycle (not per deal), then broken down by max depth reached.

**Exit reason classification:**
Iterates `exit_keywords` in definition order — more specific patterns must appear before general ones to avoid substring false-positives (e.g. `"stop"` inside `"breakeven stop"`). Falls back to profit-sign if no keyword matches.

**Loss sequence detection:**
Consecutive closed deals where P/L < 0 (minimum length 2). Captures clusters of losses better than any single worst-trade metric.

---

## Optimization Pipeline

### Native background launch

The optimizer starts `terminal64.exe /config:<path>` directly and drops the Rust child handle without terminating the process. Job metadata records the PID, report path, parameters, and start time under `%TEMP%\.mt5_mcp_quant_jobs`, allowing status polling after an MCP client reconnects. `tasklist` checks PID liveness and `taskkill` handles explicit cancellation or stale processes.

---

### `OptMode` state machine

`terminal.ini` contains an `OptMode` key that MT5 uses to track optimization state:

| `OptMode` value | Meaning |
|----------------|---------|
| `0` | Normal backtest mode (ready) |
| `1` | Optimization in progress |
| `2` | Optimization complete — show results |
| `-1` | Optimization aborted / crashed |

After any optimization run (complete or aborted), MT5 writes `-1` or `2`. On next launch with `Optimization=2` in `backtest.ini`, MT5 reads `OptMode=-1` and exits immediately without running.

**Fix:** Before every optimization launch, force `OptMode=0` in `terminal.ini`:

The Rust pipeline reads UTF-8/UTF-16LE, patches `OptMode=0`, removes stale optimization state, and writes `terminal.ini` back as UTF-16LE.

---

## Remote Agent Architecture

MT5's distributed testing works via a custom TCP protocol. The master `terminal64.exe` listens on a port. Remote agents (`metatester64.exe`) connect and receive test configurations.

```
Windows (master)                Remote machine (agents)
terminal64.exe                  metatester64.exe × N
    │                                   │
    └──── TCP:3000 ─────────────────────┘
```

**Native Windows worker example:**
```powershell
metatester64.exe /server:WINDOWS_MASTER_IP:3000 /agents:8
```

MT5 shows remote agents in the agent manager as `Agent-0.0.0.0-PORT` entries when listening, and activates them when the remote `metatester64.exe` connects.

**Throughput:** Linear scaling with agent count. 10 local + 16 remote = 26 agents. A 17,000-combination optimization that takes 3 hours locally completes in ~70 minutes.

---

## Unattended Operation

MT5-MCP-Quant uses `terminal64.exe /config:<backtest.ini>` and does not click the Strategy Tester UI. It is suitable for an interactive Windows user session, scheduled developer workstation, or Windows VM. MetaTrader 5 is a desktop application; running it from a non-interactive Windows service/session 0 is not supported by this project.

---

## Known Limitations

**Windows-specific:**
- The MCP process and MT5 must run under the same Windows user so they resolve the same `%APPDATA%` instance.
- Broker-branded installations may not use `C:\Program Files\MetaTrader 5`; `setup.ps1` scans immediate Program Files/LocalAppData directories and accepts explicit paths.
- Windows service/session-0 execution is not supported; use an interactive user session or VM login.

**Report format dependency:**
- SpreadsheetML XML format (`.htm.xml`) has no documented schema from MetaQuotes. The parser is reverse-engineered from observed output. May break on future MT5 builds.

**Comment-based analytics:**
- Strategy-specific analytics (depth histogram, exit reason, DD cause) depend on EA comment strings. EAs that don't write structured comments will get `generic` profile results — summary metrics, session breakdown, streaks, and direction bias all still work; only keyword-classified fields fall back to `"unknown"` or profit-sign.
- Custom comment patterns can be supported by adding a new entry to `PROFILES` in `src/analytics/analyze.rs`.

**Single MT5 instance:**
- MT5 is single-instance per installation/data instance. Parallel backtests require isolated terminal installations and data instances; the current configuration intentionally runs one at a time.

---

## Claude Code Integration

Use `scripts/setup.ps1` to generate the native Windows config. Project-level agent guidance should encode:
- MT5-MCP-Quant tool names and when to use them
- Baseline tracking policy (compare to `baseline.json` before calling improvements)
- Symbol name reminders (`XAUUSD.cent` ≠ `XAUUSD`)
- Backtest constraints (model 0, UTF-16LE .set files)
