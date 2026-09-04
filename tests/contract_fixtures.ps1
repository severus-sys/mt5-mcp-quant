[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

function Assert-Contract {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

$servicePath = Join-Path $repoRoot 'mql\MT5McpQuantBridge.mq5'
$providerPath = Join-Path $repoRoot 'mql\CalendarStaticProvider.mqh'
$fixturePath = Join-Path $repoRoot 'tests\fixtures\CalendarProviderCompileEA.mq5'
foreach ($path in @($servicePath, $providerPath, $fixturePath)) {
    Assert-Contract (Test-Path -LiteralPath $path -PathType Leaf) "Missing contract fixture: $path"
}

$service = Get-Content -LiteralPath $servicePath -Raw
$provider = Get-Content -LiteralPath $providerPath -Raw
$expectedOperations = @('list_server_symbols', 'ensure_selected_exact', 'export_calendar')
$operationMatches = [regex]::Matches($service, 'operation=="([a-z_]+)"') |
    ForEach-Object { $_.Groups[1].Value } |
    Sort-Object -Unique
Assert-Contract (($operationMatches -join ',') -eq (($expectedOperations | Sort-Object) -join ',')) 'MQL Service operation allowlist changed'

foreach ($forbidden in @('WebRequest(', 'OrderSend(', 'ShellExecute', '#import', 'SymbolSelect(symbol,false)')) {
    Assert-Contract (-not $service.Contains($forbidden)) "Forbidden MQL Service capability found: $forbidden"
}

$csvColumns = 'schema_version,value_id,event_id,time_server_epoch,time_server,period_server_epoch,period_server,revision,country_id,country_code,country_name,currency,event_type,sector,frequency,time_mode,unit,importance,multiplier,digits,event_code,event_name,source_url,impact_type,actual,previous,revised_previous,forecast'
Assert-Contract ($service.Contains($csvColumns)) 'Calendar CSV v1 schema changed'
foreach ($method in @('Load(', 'ValueHistory(', 'HasEventWindow(', 'LastError(')) {
    Assert-Contract ($provider.Contains($method)) "Calendar provider method missing: $method"
}
foreach ($function in @('CsvText', 'RawCalendarValue', 'ImportanceAllowed', 'NextMonth', 'WriteCalendarProgress', 'WriteCalendarRow', 'ExportCalendarSlice', 'HandleExportCalendar')) {
    $count = [regex]::Matches($service, "(?m)^(?:string|bool|datetime|void) $function\(").Count
    Assert-Contract ($count -eq 1) "MQL Service function $function must have exactly one definition; found $count"
}

$skillDirectories = @(Get-ChildItem -LiteralPath (Join-Path $repoRoot 'skills') -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'SKILL.md') })
Assert-Contract ($skillDirectories.Count -eq 10) "Expected exactly 10 MT5 skills, found $($skillDirectories.Count)"
Assert-Contract (($skillDirectories.Name | Sort-Object) -contains 'mt5-mcp-quant-calendar-data') 'Calendar data skill is missing'

$definitionSource = Get-Content -LiteralPath (Join-Path $repoRoot 'src\tools\definitions\mod.rs') -Raw
$handlerSource = Get-Content -LiteralPath (Join-Path $repoRoot 'src\tools\handlers\mod.rs') -Raw
$definitions = [regex]::Matches($definitionSource, '\b(?:[a-z_]+)::tool_[a-z0-9_]+\(\)')
$handlers = [regex]::Matches($handlerSource, '(?m)^\s*"([a-z0-9_]+)"\s*=>')
Assert-Contract ($definitions.Count -eq 96) "Expected 96 tool definitions, found $($definitions.Count)"
Assert-Contract ($handlers.Count -eq 96) "Expected 96 dispatch arms, found $($handlers.Count)"

$releaseWorkflow = Get-Content -LiteralPath (Join-Path $repoRoot '.github\workflows\release.yml') -Raw
Assert-Contract ($releaseWorkflow -match 'mt5-mcp-quant-windows-x64\.mcpb') 'Release workflow does not build an MCPB artifact'
Assert-Contract ($releaseWorkflow -match "manifest_version = '0\.3'") 'Release workflow does not emit an MCPB v0.3 manifest'
Assert-Contract ($releaseWorkflow -match "entry_point = 'mt5-mcp-quant\.exe'") 'MCPB binary entry point is missing'
Assert-Contract ($releaseWorkflow -match "command = '\$\{__dirname\}/mt5-mcp-quant\.exe'") 'MCPB stdio command is missing'
Assert-Contract ($releaseWorkflow -match "args = @\('--stdio'\)") 'MCPB stdio argument is missing'
Assert-Contract ($releaseWorkflow -match '@anthropic-ai/mcpb@2\.1\.2 pack') 'Release workflow does not use the pinned MCPB packer'

Write-Host 'Bridge/provider fixtures, 10 skills, and 96-tool source contract passed.' -ForegroundColor Green
