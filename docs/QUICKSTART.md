# Windows Quickstart

## Requirements

- Windows 10 or 11 x64
- Native MetaTrader 5, opened and signed in at least once
- [Rust stable](https://rust-lang.org/tools/install/) when building from source
- Visual Studio Build Tools with the [Desktop development with C++](https://learn.microsoft.com/en-us/visualstudio/install/workload-component-id-vs-build-tools?view=visualstudio) workload for the default MSVC Rust toolchain
- A Windows 10 or Windows 11 SDK selected in the Visual Studio Installer

## Configure

From PowerShell in the repository:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1
```

The setup script discovers both MT5 locations:

- `terminal_dir`: the program directory containing `terminal64.exe`
- `data_dir`: the active instance under `%APPDATA%\MetaQuotes\Terminal\<instance>`

They are intentionally separate because a normal Windows MT5 installation stores binaries and user data in different directories.

## Build

```powershell
cargo build --release
```

The binary is `target\release\mt5-mcp-quant.exe`.

On managed Windows systems where Application Control reports `os error 4551`, keep the security policy enabled and use:

```powershell
.\scripts\build-windows.ps1 -AppControlCompatible
```

That produces an optimized binary at `target\debug\deps\mt5-mcp-quant-release.exe`.

## Verify the Windows build

Run the native smoke test and the complete 92-tool contract/semantic suite:

```powershell
.\tests\integration_test.ps1 -Binary .\target\release\mt5-mcp-quant.exe
.\tests\e2e_all_tools.ps1 -Binary .\target\release\mt5-mcp-quant.exe
```

To include a real MetaEditor compile and Strategy Tester run, close MT5 first or explicitly allow the test to stop the configured instance:

```powershell
.\tests\e2e_all_tools.ps1 `
  -Binary .\target\release\mt5-mcp-quant.exe `
  -RunMt5 -KillExisting
```

## Register as an MCP server

Configure your MCP client to launch the executable with stdio transport. Example shape:

```json
{
  "mcpServers": {
    "mt5-mcp-quant": {
      "command": "C:\\absolute\\path\\to\\mt5-mcp-quant.exe",
      "args": ["--stdio"]
    }
  }
}
```

## Verify and backtest

Call `verify_setup`, then:

```text
Run a backtest on MyEA from 2025.01.01 to 2025.03.31
```

The pipeline runs: compile → clean → native MT5 backtest → extract → analyze.

See [CONFIG.md](CONFIG.md), [TROUBLESHOOTING.md](TROUBLESHOOTING.md), and [MCP_TOOLS.md](MCP_TOOLS.md).
