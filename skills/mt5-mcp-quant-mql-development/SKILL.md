---
name: mt5-mcp-quant-mql-development
description: Create, discover, copy, validate, and compile MQL5 Expert Advisors, indicators, and scripts with MT5-MCP-Quant. Use for EA source work, MetaEditor compilation, project scaffolding, or asset discovery.
---

# MT5-MCP-Quant MQL Development

Move MQL5 source from discovery or creation to a verified MetaEditor binary.

## Workflow

1. Discover with `list_experts`, `search_experts`, `list_indicators`, `search_indicators`, `list_scripts`, or `search_scripts`.
2. Use `init_project` only for a requested scaffold. Existing EA and README files are preserved; report created and skipped files.
3. Copy dependencies with `copy_indicator_to_project` or `copy_script_to_project`. A supplied target filename may include its extension and must end with exactly one extension.
4. Call `validate_ea_syntax` for fast feedback.
5. Call `compile_ea` for the authoritative MetaEditor result.
6. Confirm the returned `.ex5` path, binary size, error count, and warning count.

Use `create_set_template` when the compiled EA’s inputs need a Tester profile. Its output belongs under the configured MT5 Tester profiles directory.

## Failure handling

- Syntax validation is advisory; MetaEditor compilation is authoritative.
- On compile failure, return the actual error list and source path.
- If an EA exists only outside MT5’s Experts tree, compile by full `expert_path` so the source and dependencies can be synchronized.

## Completion

Finish with either a compiled `.ex5` and its exact path, or a bounded list of source errors that explains why compilation cannot complete.
