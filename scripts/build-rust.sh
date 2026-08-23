#!/usr/bin/env bash
# Build MT5-MCP-Quant release binary
# Output: target/release/mt5-mcp-quant

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

cd "$PROJECT_ROOT"

echo "=== MT5-MCP-Quant build ==="
echo "Root: $PROJECT_ROOT"
echo ""

cargo build --release

echo ""
echo "=== Done ==="
echo ""
ls -lh "$PROJECT_ROOT/target/release/mt5-mcp-quant"
echo ""
echo "Run:     ./target/release/mt5-mcp-quant --help"
echo "Install: cargo install --path . --force"
echo ""
