[CmdletBinding()]
param(
    [string]$Binary = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\mt5-mcp-quant.exe')
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Binary)) {
    throw "mt5-mcp-quant.exe not found: $Binary"
}

function Invoke-McpRequest {
    param(
        [Parameter(Mandatory)]
        [hashtable]$Request
    )

    $requestId = $Request.id
    $messages = @(
        (@{
            jsonrpc = '2.0'
            id = 0
            method = 'initialize'
            params = @{
                protocolVersion = '2024-11-05'
                capabilities = @{}
                clientInfo = @{ name = 'windows-integration'; version = '1.0' }
            }
        } | ConvertTo-Json -Compress -Depth 10),
        (@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = @{} } |
            ConvertTo-Json -Compress -Depth 10),
        ($Request | ConvertTo-Json -Compress -Depth 10)
    )

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = $messages | & $Binary --stdio 2>$null
    $ErrorActionPreference = $previousPreference

    foreach ($line in $output) {
        try {
            $response = $line | ConvertFrom-Json -ErrorAction Stop
        }
        catch {
            continue
        }
        if ($response.id -eq $requestId) {
            return $response
        }
    }

    throw "No MCP response received: id=$requestId"
}

$toolsResponse = Invoke-McpRequest @{
    jsonrpc = '2.0'; id = 1; method = 'tools/list'; params = @{}
}
if ($toolsResponse.result.tools.Count -ne 96) {
    throw "Expected exactly 96 tools, found: $($toolsResponse.result.tools.Count)"
}

$verifyResponse = Invoke-McpRequest @{
    jsonrpc = '2.0'; id = 2; method = 'tools/call'
    params = @{ name = 'verify_setup'; arguments = @{} }
}
$verify = $verifyResponse.result.content[0].text | ConvertFrom-Json
if (-not $verify.all_ok) {
    throw "verify_setup failed: $($verifyResponse.result.content[0].text)"
}

$statusResponse = Invoke-McpRequest @{
    jsonrpc = '2.0'; id = 3; method = 'tools/call'
    params = @{ name = 'check_mt5_status'; arguments = @{} }
}
$status = $statusResponse.result.content[0].text | ConvertFrom-Json
if (-not $status.terminal_ready) {
    throw "check_mt5_status failed: $($statusResponse.result.content[0].text)"
}

$resourceResponse = Invoke-McpRequest @{
    jsonrpc = '2.0'; id = 4; method = 'tools/call'
    params = @{ name = 'check_system_resources'; arguments = @{} }
}
$resources = $resourceResponse.result.content[0].text | ConvertFrom-Json
if ($resources.cpu_cores -lt 1 -or $null -eq $resources.memory) {
    throw "check_system_resources returned incomplete output: $($resourceResponse.result.content[0].text)"
}

Write-Host "Windows MCP smoke test passed: $($toolsResponse.result.tools.Count) tools, MT5 ready." -ForegroundColor Green
