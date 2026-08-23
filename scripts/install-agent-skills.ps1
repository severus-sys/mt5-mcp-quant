[CmdletBinding()]
param(
    [ValidateSet('codex', 'claude', 'opencode', 'hermes', 'all')]
    [string[]]$Client = @('all'),
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$sourceRoot = Join-Path $repoRoot 'skills'
if (-not (Test-Path -LiteralPath $sourceRoot -PathType Container)) {
    throw "Skill source directory not found: $sourceRoot"
}

$profileRoot = [Environment]::GetFolderPath('UserProfile')
if ([string]::IsNullOrWhiteSpace($profileRoot)) {
    throw 'Could not resolve the Windows user profile directory.'
}

$selected = if ($Client -contains 'all') {
    @('codex', 'claude', 'opencode', 'hermes')
}
else {
    @($Client | Select-Object -Unique)
}

$targetMap = @{
    codex = Join-Path (Join-Path $profileRoot '.codex') 'skills'
    claude = Join-Path (Join-Path $profileRoot '.claude') 'skills'
    opencode = Join-Path (Join-Path $profileRoot '.agents') 'skills'
    hermes = Join-Path (Join-Path $profileRoot '.hermes') 'skills'
}

$skillDirectories = @(Get-ChildItem -LiteralPath $sourceRoot -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'SKILL.md') })
if ($skillDirectories.Count -eq 0) {
    throw "No SKILL.md packages found under: $sourceRoot"
}

$installed = 0
$skipped = 0
foreach ($clientName in $selected) {
    $targetRoot = [IO.Path]::GetFullPath($targetMap[$clientName])
    New-Item -ItemType Directory -Force -Path $targetRoot | Out-Null

    foreach ($skill in $skillDirectories) {
        $destination = Join-Path $targetRoot $skill.Name
        if ((Test-Path -LiteralPath $destination) -and -not $Force) {
            Write-Host "SKIP [$clientName] $($skill.Name) already exists (use -Force to update)"
            $skipped++
            continue
        }

        New-Item -ItemType Directory -Force -Path $destination | Out-Null
        Get-ChildItem -LiteralPath $skill.FullName -Force |
            Copy-Item -Destination $destination -Recurse -Force
        Write-Host "INSTALLED [$clientName] $($skill.Name)"
        $installed++
    }
}

Write-Host "Agent skills complete: installed=$installed skipped=$skipped"
Write-Host 'Restart the client (or reload its skill catalog) before testing implicit routing.'
