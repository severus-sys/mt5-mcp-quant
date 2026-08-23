---
name: mt5-mcp-quant-setfiles
description: Manage MT5 UTF-16LE Tester .set files, including listing, reading, writing, patching, cloning, diffing, sweep inspection, and generation from optimization results. Use when the user mentions inputs, parameters, presets, sweeps, or .set files.
---

# MT5-MCP-Quant Set Files

Produce MT5-compatible parameter files without losing optimization flags.

## Workflow

1. Locate candidates with `list_set_files`.
2. Inspect with `read_set_file` and `describe_sweep`.
3. Use `clone_set_file` before exploratory changes when the original should remain stable.
4. Use `patch_set_file` for focused edits, `write_set_file` for a complete replacement, and `diff_set_files` to verify changes.
5. Use `set_from_optimization` to turn a winning pass into a clean backtest set or a narrowed follow-up sweep.

An optimization parameter object has this shape:

```json
{"value": 1.5, "from": 1.0, "step": 0.25, "to": 2.0, "optimize": true}
```

Scalar patches preserve an existing sweep. An object with `optimize: false` removes the sweep and keeps the selected value.

## Invariants

- Writes must remain UTF-16LE with BOM and Windows read-only attribute.
- Optimization syntax is `value||from||step||to||Y`.
- Verify total combinations before launching optimization.
- Preserve EA-agnostic parameter names and types.

## Completion

Return the output path, changed parameters, sweep dimensions, and total combinations. Re-read or diff the file so the result is verified rather than assumed.
