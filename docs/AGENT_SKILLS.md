# MT5-MCP-Quant Agent Skills

MT5-MCP-Quant ships nine portable Agent Skills:

- `mt5-mcp-quant-router` handles vague requests and chooses a workflow.
- Eight domain skills cover setup, MQL development, set files, backtesting, optimization, reports, analytics, and recovery.

The skills teach workflows; the MCP server still exposes all 92 tools.

## Install

From the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install-agent-skills.ps1 -Client all
```

Use `-Force` to update an existing installation.

| Client | Installation target |
|---|---|
| Codex / ChatGPT coding agent | `%USERPROFILE%\.codex\skills` |
| Claude Code | `%USERPROFILE%\.claude\skills` |
| OpenCode | `%USERPROFILE%\.agents\skills` |
| Hermes | `%USERPROFILE%\.hermes\skills` |

The canonical source remains the repository `skills/` directory. Hermes can also consume that directory as a Skills Hub tap, and Claude/Codex plugins can package it as their plugin `skills/` directory.

Restart the client or reload its skill catalog after installation. The MCP server should be registered as `mt5_mcp_quant`; a different local server name is acceptable when it exposes the same 92 MT5-MCP-Quant tools.

## Vague prompts

The router is deliberately available for implicit invocation. A user does not need to know tool names.

Example:

```text
MT5 MCP'yi kullan ve EA'me bak.
```

Default behavior:

1. Check MCP/MT5 health and the active account.
2. Discover available EAs.
3. Inspect the latest report.
4. Infer whether development, backtesting, optimization, reporting, analytics, or recovery is the useful route.
5. Continue with the safe first action, or ask one outcome-level question when two materially different routes remain.

More specific prompts route directly:

- “EA’yı derle” → MQL development
- “Geçen ay test et” → backtesting
- “En iyi ayarları bul” → optimization
- “Neden zarar etti?” → analytics
- “MT5 takıldı” → debug and recovery

## Validate

Each skill must pass the Codex skill validator:

```powershell
$validator = Join-Path $env:USERPROFILE '.codex\skills\.system\skill-creator\scripts\quick_validate.py'
Get-ChildItem skills -Directory | ForEach-Object {
    python $validator $_.FullName
}
```

Behavioral routing tests should cover:

- a fully vague request;
- an EA name without an operation;
- explicit backtest, optimization, analysis, and failure intents;
- a request that would require destructive cleanup or process termination.
