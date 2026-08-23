[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [switch]$AppControlCompatible
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    if ($DebugBuild -and $AppControlCompatible) {
        throw 'Use either -DebugBuild or -AppControlCompatible, not both.'
    }

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found. Open a new PowerShell window after installing Rust.'
    }

    cargo check --all-targets
    cargo test --all-targets
    if ($DebugBuild) {
        cargo build
        Write-Host "Ready: $repoRoot\target\debug\mt5-mcp-quant.exe" -ForegroundColor Green
    }
    elseif ($AppControlCompatible) {
        $previousTargetDir = $env:CARGO_TARGET_DIR
        $targetRoot = Join-Path $repoRoot 'target\debug\deps\release-build'
        try {
            $env:CARGO_TARGET_DIR = $targetRoot
            cargo build --release
            if ($LASTEXITCODE -ne 0) {
                throw "cargo build --release failed with exit code $LASTEXITCODE"
            }
        }
        finally {
            $env:CARGO_TARGET_DIR = $previousTargetDir
        }

        $builtBinary = Join-Path $targetRoot 'release\mt5-mcp-quant.exe'
        $outputBinary = Join-Path $repoRoot 'target\debug\deps\mt5-mcp-quant-release.exe'
        Copy-Item -LiteralPath $builtBinary -Destination $outputBinary -Force
        Write-Host "Ready: $outputBinary" -ForegroundColor Green
    }
    else {
        cargo build --release
        Write-Host "Ready: $repoRoot\target\release\mt5-mcp-quant.exe" -ForegroundColor Green
    }
}
finally {
    Pop-Location
}
