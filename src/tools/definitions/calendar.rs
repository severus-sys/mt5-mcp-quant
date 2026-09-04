use serde_json::{json, Value};

pub fn tool_prepare_calendar_export() -> Value {
    json!({
        "name": "prepare_calendar_export",
        "description": "Create an idempotent asynchronous export job for the active broker's live MT5 economic calendar. Times are broker server-time values without a timezone suffix.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "currencies": { "type": "array", "items": { "type": "string", "pattern": "^[A-Za-z]{3}$" }, "default": [] },
                "country_codes": { "type": "array", "items": { "type": "string", "pattern": "^[A-Za-z]{2}$" }, "default": [] },
                "importance": { "type": "array", "items": { "type": "string", "enum": ["low", "moderate", "high"] }, "default": ["high"] },
                "from": { "type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}$", "description": "Inclusive broker server-time start." },
                "to": { "type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}$", "description": "Exclusive broker server-time end." },
                "output_format": { "type": "string", "enum": ["csv"], "default": "csv" },
                "overwrite": { "type": "boolean", "default": false }
            },
            "required": ["from", "to"],
            "additionalProperties": false
        }
    })
}

pub fn tool_inspect_calendar_export() -> Value {
    json!({
        "name": "inspect_calendar_export",
        "description": "Inspect a persistent calendar export job. Polling is idempotent and reports progress, validation, coverage, filters, row count, and machine-readable errors.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "pattern": "^[A-Za-z0-9_-]{1,64}$" },
                "validate_rows": { "type": "boolean", "default": true }
            },
            "required": ["job_id"],
            "additionalProperties": false
        }
    })
}

pub fn tool_prepare_calendar_backtest_dataset() -> Value {
    json!({
        "name": "prepare_calendar_backtest_dataset",
        "description": "Publish a validated calendar export as an immutable FILE_COMMON CSV v1 dataset plus checksummed manifest for Strategy Tester EAs.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "pattern": "^[A-Za-z0-9_-]{1,64}$" },
                "dataset_name": { "type": "string", "pattern": "^[A-Za-z0-9._-]{1,64}$" },
                "overwrite": { "type": "boolean", "default": false }
            },
            "required": ["job_id", "dataset_name"],
            "additionalProperties": false
        }
    })
}
