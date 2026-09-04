---
name: mt5-mcp-quant-calendar-data
description: Export the live MT5 economic calendar and publish reproducible static Strategy Tester datasets. Use for economic-calendar research, news filters, historical events, static tester data, CalendarValueHistory, or MT5 tester error 4014.
---

# MT5-MCP-Quant Calendar Data

Create broker-specific, auditable calendar datasets without calling the live calendar API from Strategy Tester.

## Workflow

1. Call `verify_setup` and require `mql_bridge.state=ready` for automatic export. If the bridge is only installed, ask the user to start `MT5McpQuantBridge` once from MT5 Navigator → Services → MT5-MCP-Quant.
2. Call `prepare_calendar_export` with currency and/or country filters, importance, and an inclusive/exclusive broker-server-time range.
3. Poll `inspect_calendar_export` with the returned `job_id`. Polling is idempotent. Continue only after `validated` or `partial`; never treat `failed` or `invalid` as usable data.
4. Report observed coverage when the broker returns only part of the requested history. Do not label a partial export complete.
5. Call `prepare_calendar_backtest_dataset` with a stable dataset name.
6. In the EA, include `<MT5-MCP-Quant/CalendarStaticProvider.mqh>`, create `CMt5MqCalendarStaticProvider`, call `Load(dataset_id)`, and query `ValueHistory` or `HasEventWindow`.

## Filter and time rules

- At least one currency or country is required.
- Values within currencies and countries are OR filters. When both categories are supplied, the categories are combined with AND.
- Dates use `YYYY-MM-DDTHH:MM:SS` in the active broker's server time. Never append `Z` or a UTC offset.
- The exporter stores MT5's raw scaled `int64` values. An empty CSV field means MT5 `LONG_MIN`/missing; numeric zero is a real value.
- Rows are deterministic and deduplicated by `value_id`.

## Safety and broker identity

- Dataset manifests bind schema, checksum, account server, terminal instance, filters, coverage, and source job.
- `Load` rejects schema or checksum mismatches.
- It also rejects a different broker/server by default. Use `allow_broker_mismatch=true` only when the user explicitly accepts the comparability risk; surface the provider's warning.
- Jobs and datasets are persistent. Never remove them as cleanup unless the user explicitly requests deletion.

## Error routing

- MT5 tester error 4014 or a live-calendar call failing inside an EA: use this static export workflow; do not automate a per-export Script.
- `prepared`: the job exists, but the Service is not ready. Start the Service once and repeat the same prepare call; its fingerprint makes that safe.
- `partial`: dataset publication is allowed, but report requested versus observed coverage.
- `invalid`: inspect the machine-readable error; do not publish.
- `protocol_mismatch`, `wrong_terminal_instance`, or `stale`: route to `mt5-mcp-quant-setup-health` before retrying.

## Completion

Finish when the export is structurally validated, the dataset and checksum manifest are published, the provider include is deployed, and the user sees the dataset ID, broker identity, row count, completeness, and coverage.
