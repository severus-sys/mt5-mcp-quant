[CmdletBinding()]
param(
    [string]$Binary = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\mt5-mcp-quant.exe'),
    [switch]$RunMt5,
    [switch]$KillExisting,
    [string]$Symbol,
    [string]$FromDate = '2025.01.06',
    [string]$ToDate = '2025.01.07',
    [switch]$KeepArtifacts
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$Binary = [IO.Path]::GetFullPath($Binary)
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "mt5-mcp-quant.exe not found: $Binary"
}

$expectedTools = @(
    'analyze_concurrent_peak', 'analyze_costs', 'analyze_direction_bias',
    'analyze_drawdown_events', 'analyze_efficiency', 'analyze_hold_time_distribution',
    'analyze_layer_performance', 'analyze_loss_sequences', 'analyze_monthly_pnl',
    'analyze_position_pairs', 'analyze_profit_distribution', 'analyze_report',
    'analyze_streaks', 'analyze_time_performance', 'analyze_top_losses',
    'analyze_volume_vs_profit', 'annotate_history', 'archive_all_reports',
    'archive_report', 'cache_status', 'check_mt5_process', 'check_mt5_status',
    'check_symbol_data_status', 'check_system_resources', 'check_update', 'clean_cache',
    'clone_set_file', 'compare_backtests', 'compare_baseline', 'compile_ea',
    'copy_indicator_to_project', 'copy_script_to_project', 'create_set_template',
    'describe_sweep', 'diagnose_wine', 'diff_set_files', 'export_deals_csv',
    'export_report', 'get_active_account', 'get_backtest_crash_info',
    'get_backtest_history', 'get_backtest_status', 'get_best_reports',
    'get_comparable_reports', 'get_history', 'get_latest_report', 'get_mt5_logs',
    'get_optimization_results', 'get_optimization_status', 'get_report_by_id',
    'get_reports_by_set_file', 'get_reports_summary', 'get_tester_log',
    'get_wine_prefix_info', 'healthcheck', 'init_project', 'kill_mt5_process',
    'launch_backtest', 'list_deals', 'list_experts', 'list_indicators', 'list_jobs',
    'list_reports', 'list_scripts', 'list_set_files', 'list_symbols', 'patch_set_file',
    'promote_to_baseline', 'prune_reports', 'read_set_file', 'run_backtest',
    'run_backtest_only', 'run_backtest_quick', 'run_optimization',
    'run_rolling_backtest', 'search_deals_by_comment', 'search_deals_by_magic',
    'search_experts', 'search_indicators', 'search_mt5_errors', 'search_reports',
    'search_reports_by_date_range', 'search_reports_by_notes',
    'search_reports_by_tags', 'search_scripts', 'set_from_optimization', 'tail_log',
    'update', 'validate_ea_syntax', 'validate_mt5_config', 'verify_setup',
    'write_set_file'
) | Sort-Object

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("mt5-mcp-quant-e2e-" + [guid]::NewGuid().ToString('N'))
$testHome = Join-Path $testRoot 'home'
$setDir = Join-Path $testRoot 'sets'
New-Item -ItemType Directory -Force -Path $testHome, $setDir | Out-Null

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Invoke-McpRequest {
    param([Parameter(Mandatory)][hashtable]$Request)

    $messages = @(
        (@{
            jsonrpc = '2.0'; id = 0; method = 'initialize'
            params = @{
                protocolVersion = '2024-11-05'; capabilities = @{}
                clientInfo = @{ name = 'windows-all-tools-e2e'; version = '1.0' }
            }
        } | ConvertTo-Json -Compress -Depth 20),
        (@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = @{} } |
            ConvertTo-Json -Compress -Depth 20),
        ($Request | ConvertTo-Json -Compress -Depth 30)
    )

    $previousHome = $env:MT5_MCP_QUANT_HOME
    $previousErrorActionPreference = $ErrorActionPreference
    $env:MT5_MCP_QUANT_HOME = $testHome
    $ErrorActionPreference = 'Continue'
    try {
        $output = $messages | & $Binary --stdio 2>$null
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
        $env:MT5_MCP_QUANT_HOME = $previousHome
    }
    Assert-True ($exitCode -eq 0) "MCP process exited with code $exitCode"

    foreach ($line in $output) {
        try { $response = $line | ConvertFrom-Json -ErrorAction Stop }
        catch { continue }
        if ($response.id -eq $Request.id) { return $response }
    }
    throw "No response for request id=$($Request.id)"
}

function Invoke-Tool {
    param([string]$Name, [hashtable]$Arguments = @{}, [int]$Id = 10)
    Invoke-McpRequest @{
        jsonrpc = '2.0'; id = $Id; method = 'tools/call'
        params = @{ name = $Name; arguments = $Arguments }
    }
}

function Get-Payload {
    param($Response, [string]$Tool)
    if ($Response.error) { throw "$Tool JSON-RPC error: $($Response.error.message)" }
    if ($Response.result.isError) { throw "$Tool tool error: $($Response.result.content[0].text)" }
    $Response.result.content[0].text | ConvertFrom-Json
}

try {
    Write-Host '1/5 Exact tool inventory and schema validation'
    $toolsResponse = Invoke-McpRequest @{
        jsonrpc = '2.0'; id = 1; method = 'tools/list'; params = @{}
    }
    Assert-True (-not $toolsResponse.error) "tools/list failed: $($toolsResponse.error.message)"
    $tools = @($toolsResponse.result.tools)
    $actualNames = @($tools.name | Sort-Object)
    Assert-True ($actualNames.Count -eq 92) "Expected exactly 92 tools, found $($actualNames.Count)"
    Assert-True ((@($actualNames | Select-Object -Unique)).Count -eq 92) 'Tool names are not unique'
    Assert-True (($actualNames -join "`n") -eq ($expectedTools -join "`n")) 'Exact 92-tool inventory changed'
    foreach ($tool in $tools) {
        Assert-True ($tool.inputSchema.type -eq 'object') "$($tool.name) has an invalid input schema"
    }

    Write-Host '2/5 Dispatch validation for all 92 tools'
    $safeArguments = @{
        update = @{ dry_run = $true }
        kill_mt5_process = @{ pid = 'not-a-pid'; force = $false }
        clean_cache = @{ symbol = '__MT5_MCP_QUANT_NO_SUCH_SYMBOL__'; dry_run = $true }
        prune_reports = @{ keep_last = 10 }
        search_experts = @{ pattern = '__none__' }
        search_indicators = @{ pattern = '__none__' }
        search_scripts = @{ pattern = '__none__' }
        search_deals_by_comment = @{ query = '__none__' }
        search_deals_by_magic = @{ magic = '__none__' }
        compare_baseline = @{ baseline = '__missing__' }
        check_symbol_data_status = @{ symbol = '__none__'; from_date = $FromDate; to_date = $ToDate }
        compare_backtests = @{ report_dirs = @() }
        search_reports_by_tags = @{ tags = @('__none__') }
        search_reports_by_notes = @{ query = '__none__' }
        get_reports_by_set_file = @{ set_file = '__none__' }
    }
    $requestId = 100
    foreach ($name in $expectedTools) {
        $arguments = if ($safeArguments.ContainsKey($name)) { $safeArguments[$name] } else { @{} }
        $response = Invoke-Tool -Name $name -Arguments $arguments -Id $requestId
        if ($response.error -and $response.error.message -match 'Unknown tool|Method not found') {
            throw "$name is listed but not dispatched: $($response.error.message)"
        }
        $requestId++
    }

    Write-Host '3/5 Native Windows diagnostics and read-only behavior'
    $verify = Get-Payload (Invoke-Tool 'verify_setup') 'verify_setup'
    Assert-True $verify.all_ok "verify_setup reported failure"
    $status = Get-Payload (Invoke-Tool 'check_mt5_status') 'check_mt5_status'
    Assert-True $status.terminal_ready 'MT5 is not ready'
    $terminalLog = Get-Payload (Invoke-Tool 'get_mt5_logs' @{ log_type = 'terminal'; lines = 3 }) 'get_mt5_logs'
    Assert-True ($terminalLog.found -and $terminalLog.lines_returned -gt 0) 'Terminal log was not decoded'
    $testerLog = Get-Payload (Invoke-Tool 'get_mt5_logs' @{ log_type = 'tester'; lines = 3 }) 'get_mt5_logs'
    Assert-True ($testerLog.found -and $testerLog.lines_returned -gt 0) 'Tester log was not decoded'
    $testerJournal = Get-Payload (Invoke-Tool 'get_tester_log' @{ tail_lines = 3 }) 'get_tester_log'
    Assert-True ($testerJournal.total_lines -gt 0) 'Native tester journal was not found'
    $configValidation = Get-Payload (Invoke-Tool 'validate_mt5_config') 'validate_mt5_config'
    Assert-True ($configValidation.config_files_found -contains 'terminal.ini') 'config\terminal.ini was not found'
    $cache = Get-Payload (Invoke-Tool 'cache_status') 'cache_status'
    Assert-True (-not ($cache.symbols -contains 'cache')) 'cache_status reported a directory as a symbol'
    $cacheDryRun = Get-Payload (Invoke-Tool 'clean_cache' @{ symbol = '__MT5_MCP_QUANT_NO_SUCH_SYMBOL__'; dry_run = $true }) 'clean_cache'
    Assert-True ($cacheDryRun.bytes_freed -eq 0) 'Per-symbol cache filtering is not working'
    $updateDryRun = Get-Payload (Invoke-Tool 'update' @{ dry_run = $true }) 'update'
    Assert-True $updateDryRun.dry_run 'Update dry-run unexpectedly changed state'

    Write-Host '4/5 .set and optimization result round trips'
    $setA = Join-Path $setDir 'strategy-a.set'
    $setB = Join-Path $setDir 'strategy-b.set'
    Get-Payload (Invoke-Tool 'write_set_file' @{
        path = $setA
        parameters = @{
            Risk = @{ value = 1.5; optimize = $false }
            Sweep = @{ value = 2; from = 1; step = 1; to = 4; optimize = $true }
            Enabled = $true
        }
    }) 'write_set_file' | Out-Null
    Get-Payload (Invoke-Tool 'patch_set_file' @{ path = $setA; patches = @{ Risk = 2.5; Sweep = 3 } }) 'patch_set_file' | Out-Null
    $readSet = Get-Payload (Invoke-Tool 'read_set_file' @{ path = $setA }) 'read_set_file'
    Assert-True ($readSet.parameters.Risk.value -eq '2.5') 'Numeric .set patch failed'
    Assert-True $readSet.parameters.Sweep.optimize 'Patch removed the sweep range'
    Get-Payload (Invoke-Tool 'clone_set_file' @{ source = $setA; destination = $setB }) 'clone_set_file' | Out-Null
    Get-Payload (Invoke-Tool 'patch_set_file' @{ path = $setB; patches = @{ Extra = 9 } }) 'patch_set_file' | Out-Null
    $diff = Get-Payload (Invoke-Tool 'diff_set_files' @{ file_a = $setA; file_b = $setB }) 'diff_set_files'
    Assert-True ($diff.total_differences -gt 0) 'Diff missed an extra trailing parameter'

    $optimizedSet = Join-Path $setDir 'optimized.set'
    $setResult = Get-Payload (Invoke-Tool 'set_from_optimization' @{
        path = $optimizedSet
        template = $setA
        params = @{ Risk = 3.25; Enabled = $false }
        sweep = @{ Risk = @{ from = 2.5; step = 0.25; to = 3.5 } }
    }) 'set_from_optimization'
    Assert-True ($setResult.opt_params_applied -eq 2) 'Optimization params were dropped'
    Assert-True ($setResult.swept_params -eq 1 -and $setResult.total_combinations -eq 5) 'Sweep combinations are incorrect'

    $optimizationFixture = Join-Path $repoRoot 'tests\fixtures\sample_optimization.xml'
    $optimization = Get-Payload (Invoke-Tool 'get_optimization_results' @{ report_file = $optimizationFixture; top_n = 1 }) 'get_optimization_results'
    Assert-True ($optimization.top_passes[0].params.TP_Pips -eq '400') 'SpreadsheetML EA parameter was lost'
    Assert-True ($optimization.top_passes[0].params.UseFilter -eq 'true') 'SpreadsheetML boolean parameter was lost'

    if ($RunMt5) {
        Write-Host '5/5 Real MetaEditor compile and Strategy Tester pipeline'
        $fixtureEa = Join-Path $repoRoot 'tests\fixtures\WindowsSmokeEA.mq5'
        $compile = Get-Payload (Invoke-Tool 'compile_ea' @{ expert_path = $fixtureEa }) 'compile_ea'
        Assert-True $compile.success "MetaEditor compilation failed"

        if (-not $Symbol) {
            $symbols = Get-Payload (Invoke-Tool 'list_symbols') 'list_symbols'
            $Symbol = if ($symbols.symbols -contains 'EURUSD') { 'EURUSD' } else { $symbols.symbols[0] }
        }
        Assert-True (-not [string]::IsNullOrWhiteSpace($Symbol)) 'No tester symbol is available'

        $backtest = Get-Payload (Invoke-Tool 'run_backtest' @{
            expert = 'WindowsSmokeEA'; symbol = $Symbol
            from_date = $FromDate; to_date = $ToDate; timeframe = 'M5'
            model = 0; skip_compile = $true; skip_clean = $true
            kill_existing = [bool]$KillExisting; shutdown = $true
            timeout = 300; startup_delay_secs = 5
        }) 'run_backtest'
        Assert-True $backtest.success 'Strategy Tester pipeline failed'
        Assert-True (Test-Path -LiteralPath (Join-Path $backtest.report_dir 'analysis.json')) 'Analysis output is missing'
    }
    else {
        Write-Host '5/5 Stateful MT5 compile/backtest skipped (use -RunMt5 -KillExisting)'
    }

    Write-Host "PASS: exact 92-tool contract and Windows semantic E2E completed." -ForegroundColor Green
}
finally {
    if ($KeepArtifacts) {
        Write-Host "E2E artifacts kept at $testRoot"
    }
    else {
        $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
        $resolvedTempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if (-not $resolvedTestRoot.StartsWith($resolvedTempRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove non-temp E2E path: $resolvedTestRoot"
        }
        if (Test-Path -LiteralPath $resolvedTestRoot) {
            Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
        }
    }
}
