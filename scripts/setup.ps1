[CmdletBinding()]
param(
    [string]$TerminalDir,
    [string]$DataDir,
    [string]$ProjectDir,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$installationRoot = if ($env:MT5_MCP_QUANT_HOME) {
    $env:MT5_MCP_QUANT_HOME
} else {
    Join-Path $env:APPDATA 'mt5-mcp-quant'
}
$configPath = Join-Path $installationRoot 'config\mt5-mcp-quant.yaml'

function Find-Mt5Install {
    $roots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:LOCALAPPDATA) |
        Where-Object { $_ -and (Test-Path -LiteralPath $_) } |
        Select-Object -Unique

    $candidates = foreach ($root in $roots) {
        $default = Join-Path $root 'MetaTrader 5'
        if (Test-Path -LiteralPath (Join-Path $default 'terminal64.exe')) {
            Get-Item -LiteralPath $default
        }
        Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'terminal64.exe') }
    }

    $candidates | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}

function Find-Mt5Data([string]$InstallPath) {
    if ($InstallPath -and (Test-Path -LiteralPath (Join-Path $InstallPath 'MQL5'))) {
        return $InstallPath
    }

    $instancesRoot = Join-Path $env:APPDATA 'MetaQuotes\Terminal'
    if (-not (Test-Path -LiteralPath $instancesRoot)) {
        return $null
    }

    $instances = Get-ChildItem -LiteralPath $instancesRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'MQL5') }

    if ($InstallPath) {
        $expected = [IO.Path]::GetFullPath($InstallPath).TrimEnd('\').ToLowerInvariant()
        foreach ($instance in $instances) {
            $originPath = Join-Path $instance.FullName 'origin.txt'
            if (-not (Test-Path -LiteralPath $originPath)) { continue }
            $origin = (Get-Content -Raw -LiteralPath $originPath).Trim([char]0).Trim().TrimEnd('\').ToLowerInvariant()
            if ($origin -eq $expected) { return $instance.FullName }
        }
    }

    $instances | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}

function Quote-Yaml([string]$Value) {
    if ($null -eq $Value) { return '~' }
    "'" + $Value.Replace("'", "''") + "'"
}

if (-not $TerminalDir) { $TerminalDir = Find-Mt5Install }
if (-not $TerminalDir -or -not (Test-Path -LiteralPath (Join-Path $TerminalDir 'terminal64.exe'))) {
    throw 'terminal64.exe was not found. Pass the MetaTrader 5 install directory with -TerminalDir.'
}

if (-not $DataDir) { $DataDir = Find-Mt5Data $TerminalDir }
if (-not $DataDir -or -not (Test-Path -LiteralPath (Join-Path $DataDir 'MQL5'))) {
    throw 'The MT5 data directory was not found. Open MT5 once or pass %APPDATA%\MetaQuotes\Terminal\<instance> with -DataDir.'
}

if (-not $ProjectDir) { $ProjectDir = $repoRoot }
$expertsDir = Join-Path $DataDir 'MQL5\Experts'
$indicatorsDir = Join-Path $DataDir 'MQL5\Indicators'
$scriptsDir = Join-Path $DataDir 'MQL5\Scripts'
$servicesDir = Join-Path $DataDir 'MQL5\Services'
$includeDir = Join-Path $DataDir 'MQL5\Include'
$terminalCommonDataDir = Join-Path (Split-Path -Parent $DataDir) 'Common'
$profilesDir = Join-Path $DataDir 'MQL5\Profiles\Tester'
$testerDir = Join-Path $DataDir 'Tester'
$reportsDir = Join-Path $repoRoot 'reports'
$logDir = Join-Path $env:TEMP 'mt5-mcp-quant\logs'

if ((Test-Path -LiteralPath $configPath) -and -not $Force) {
    Write-Host "Config already exists: $configPath"
    Write-Host 'Use -Force to overwrite it.'
    exit 0
}

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $configPath), $profilesDir, $servicesDir, $includeDir, $terminalCommonDataDir, $reportsDir, $logDir | Out-Null

$yaml = @"
# mt5-mcp-quant native Windows configuration
terminal_dir: $(Quote-Yaml $TerminalDir)
data_dir: $(Quote-Yaml $DataDir)
experts_dir: $(Quote-Yaml $expertsDir)
indicators_dir: $(Quote-Yaml $indicatorsDir)
scripts_dir: $(Quote-Yaml $scriptsDir)
services_dir: $(Quote-Yaml $servicesDir)
include_dir: $(Quote-Yaml $includeDir)
terminal_common_data_dir: $(Quote-Yaml $terminalCommonDataDir)
tester_profiles_dir: $(Quote-Yaml $profilesDir)
tester_cache_dir: $(Quote-Yaml $testerDir)
display_mode: gui
project_dir: $(Quote-Yaml $ProjectDir)

backtest_symbol: EURUSD
backtest_deposit: 10000
backtest_currency: USD
backtest_leverage: 100
backtest_model: 0
backtest_timeframe: H1
backtest_timeout: 900

opt_log_dir: $(Quote-Yaml $logDir)
opt_min_agents: 1
opt_max_agents: 0
reports_dir: $(Quote-Yaml $reportsDir)
"@

$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($configPath, $yaml, $utf8WithoutBom)

Write-Host 'MT5-MCP-Quant native Windows configuration is ready.' -ForegroundColor Green
Write-Host "  MT5 program: $TerminalDir"
Write-Host "  MT5 data:    $DataDir"
Write-Host "  Config:      $configPath"
Write-Host ''
Write-Host 'Next steps:'
Write-Host '  1. cargo build --release'
Write-Host '  2. target\release\mt5-mcp-quant.exe --stdio'
Write-Host '  3. Run the verify_setup tool from your MCP client.'
