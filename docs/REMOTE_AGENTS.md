# Remote Agent Setup (Windows Master)

MT5's distributed testing lets a native Windows master distribute optimization passes to additional MetaTester agents. The MCP server controls the Windows `terminal64.exe`; remote-agent lifecycle remains an MT5 infrastructure concern.

**Throughput:** Each agent handles one pass at a time. 10 local + 16 remote = 26 agents. A 17,000-combination genetic optimization that takes 3 hours locally finishes in ~70 minutes.

---

## Requirements

**Windows (master)**
- Native MetaTrader 5 installed and signed in
- MT5-MCP-Quant configured and working for local backtests
- The selected agent port allowed through Windows Defender Firewall

**Linux server (agents)**
- Wine 7.0+
- `metatester64.exe` from an MT5 installation
- Access to the same MT5 data files (tick history) as the master

## Step 1: Find the MT5 agent port on Windows

After running a local optimization, check the MT5 agent directories:

Inspect the configured `data_dir` and the Strategy Tester **Agents** tab. Local agent directories are under `<data_dir>\Tester`.

You'll see directories like:
```
Agent-127.0.0.1-3000/   ← local agents (loopback)
Agent-0.0.0.0-3000/     ← remote listener (if enabled)
```

If you don't see `Agent-0.0.0.0-*` directories, enable remote agents in MT5:
**Tools → Options → Expert Advisors → Allow remote agents**

Note the port number (default: 3000).

## Step 2: Provision remote workers

For native Windows workers, install the same MT5 build and configure its MetaTester agents. Linux/Wine workers remain possible but are outside the native Windows MCP runtime.
```bash
WINDOWS_MT5="/path/to/copied/windows/mt5/files"

scp "${WINDOWS_MT5}/metatester64.exe" user@linux-server:~/mt5agents/
scp "${WINDOWS_MT5}"/*.dll user@linux-server:~/mt5agents/
```

## Step 3: Launch agents on Linux

```bash
cd ~/mt5agents/
wine64 metatester64.exe /server:192.168.1.100:3000 /agents:8
```

**To run as a background service:**
```bash
nohup wine64 metatester64.exe /server:192.168.1.100:3000 /agents:8 \
    > ~/mt5agents/agents.log 2>&1 &
disown
```

## Step 4: Verify agents appear in MT5

On the Windows master, open MT5:
**View → Strategy Tester → Agents tab**

You should see entries like:
```
Agent-192.168.1.200-3000  [Active]
```

## Step 5: Configure MT5-MCP-Quant for remote agents

In `config/mt5-mcp-quant.yaml`:
```yaml
optimization:
  remote_agents:
    enabled: true
    check_agent_count: true
    min_agents: 4
```

## Tick Data Sync

On first run, MT5 automatically downloads ticks from the broker. Pre-populate on Linux:

```bash
# On Windows — locate tick data below the configured data_dir

# Copy to Linux
scp -r "${TICK_DIR}" user@linux-server:~/mt5agents/ticks/
```

## Troubleshooting

**Agents connect then immediately disconnect**
- MT5 version mismatch between `metatester64.exe` (from Mac) and the master. Use the exact same build number.

**Agents show as connected but don't receive work**
- Start an optimization from the MT5 GUI first to "activate" remote agents, then cancel it and use MT5-MCP-Quant.

**Performance is slower with remote agents**
- Use wired connection. WiFi or WAN: significant overhead.
