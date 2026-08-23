#!/usr/bin/env bash
# Compatibility launcher for the native Windows all-tools E2E suite.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if command -v pwsh.exe >/dev/null 2>&1; then
    exec pwsh.exe -NoProfile -File "${SCRIPT_DIR}/e2e_all_tools.ps1" "$@"
elif command -v powershell.exe >/dev/null 2>&1; then
    exec powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${SCRIPT_DIR}/e2e_all_tools.ps1" "$@"
else
    echo "This project is native Windows. Run tests/e2e_all_tools.ps1 from PowerShell." >&2
    exit 1
fi
