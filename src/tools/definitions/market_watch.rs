use serde_json::{json, Value};

pub fn tool_ensure_market_watch_symbol() -> Value {
    json!({
        "name": "ensure_market_watch_symbol",
        "description": "Resolve a requested symbol against the active broker's complete catalog and ensure the exact broker symbol is selected and visible in Market Watch. Never guesses when aliases are ambiguous and never removes symbols.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Requested symbol or base alias, for example EURUSD when the broker may expose EURUSDm."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 500,
                    "maximum": 30000,
                    "default": 5000,
                    "description": "Maximum bridge wait time in milliseconds."
                }
            },
            "required": ["symbol"],
            "additionalProperties": false
        }
    })
}
