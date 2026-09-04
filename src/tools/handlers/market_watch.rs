use crate::bridge::{BridgeClient, BridgeHealthState};
use crate::models::{resolve_symbol, Config, SymbolMatch};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

fn tool_error(body: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": body.to_string() }],
        "isError": true
    })
}

pub async fn handle_ensure_market_watch_symbol(config: &Config, args: &Value) -> Result<Value> {
    let requested = args
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("symbol is required and cannot be empty"))?;
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(5_000);
    if !(500..=30_000).contains(&timeout_ms) {
        return Ok(tool_error(json!({
            "code": "invalid_timeout",
            "error": "timeout_ms must be between 500 and 30000.",
            "requested_symbol": requested,
        })));
    }

    let bridge = match BridgeClient::new(config) {
        Ok(bridge) => bridge,
        Err(error) => {
            return Ok(tool_error(json!({
                "code": "bridge_not_configured",
                "error": error.to_string(),
                "requested_symbol": requested,
                "hint": "Run scripts/setup.ps1, then call verify_setup."
            })))
        }
    };
    let installation = match bridge.ensure_installed().await {
        Ok(result) => result,
        Err(error) => {
            return Ok(tool_error(json!({
                "code": "bridge_install_failed",
                "error": error.to_string(),
                "requested_symbol": requested,
                "hint": "Check verify_setup, MetaEditor logs, and write access to MQL5/Services."
            })))
        }
    };
    let health = bridge.health();
    if health.state != BridgeHealthState::Ready {
        return Ok(tool_error(json!({
            "code": "bridge_not_ready",
            "error": "The native MT5 bridge Service is installed but not ready.",
            "requested_symbol": requested,
            "bridge": health,
            "installation": installation,
            "start_service_once": [
                "Open MT5 Navigator (Ctrl+N).",
                "Expand Services > MT5-MCP-Quant.",
                "Right-click MT5McpQuantBridge and choose Start.",
                "Retry this tool after verify_setup reports mql_bridge.state=ready."
            ]
        })));
    }
    if health.connected == Some(false) {
        return Ok(tool_error(json!({
            "code": "terminal_disconnected",
            "error": "MT5 is not connected to the active broker.",
            "requested_symbol": requested,
            "bridge": health,
            "hint": "Reconnect the configured MT5 terminal to its broker, then retry."
        })));
    }

    let timeout = Duration::from_millis(timeout_ms);
    let catalog = match bridge.list_server_symbols(timeout).await {
        Ok(symbols) => symbols,
        Err(error) => {
            return Ok(tool_error(json!({
                "code": "broker_catalog_unavailable",
                "error": error.to_string(),
                "requested_symbol": requested,
                "active_server": health.account_server,
                "hint": "Confirm the Service heartbeat is fresh and MT5 is connected to the broker."
            })))
        }
    };
    let matched = resolve_symbol(requested, &catalog);
    let resolution = match &matched {
        SymbolMatch::Exact(_) => json!({ "status": "exact" }),
        SymbolMatch::Alias { kind, .. } => json!({ "status": "alias", "method": kind }),
        SymbolMatch::Ambiguous(candidates) => {
            json!({ "status": "ambiguous", "candidates": candidates })
        }
        SymbolMatch::NoMatch => json!({ "status": "no_match" }),
    };
    let Some(resolved) = matched.resolved().map(str::to_string) else {
        let ambiguous = matches!(matched, SymbolMatch::Ambiguous(_));
        return Ok(tool_error(json!({
            "code": if ambiguous { "symbol_ambiguous" } else { "symbol_not_found" },
            "error": if ambiguous {
                format!("Symbol '{}' matches multiple broker symbols; Market Watch was not changed.", requested)
            } else {
                format!("Symbol '{}' was not found in the active broker catalog.", requested)
            },
            "requested_symbol": requested,
            "resolved_symbol": null,
            "symbol_resolution": resolution,
            "candidates": matched.candidates(),
            "active_server": health.account_server,
            "catalog_size": catalog.len(),
            "hint": if ambiguous {
                "Retry with one exact candidate."
            } else {
                "Check the active account/server and use the broker's exact symbol name."
            }
        })));
    };

    let response = match bridge
        .request(
            "ensure_selected_exact",
            &BTreeMap::from([("symbol".to_string(), resolved.clone())]),
            timeout,
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Ok(tool_error(json!({
                "code": "bridge_request_failed",
                "error": error.to_string(),
                "requested_symbol": requested,
                "resolved_symbol": resolved,
                "symbol_resolution": resolution,
                "active_server": health.account_server,
                "hint": "Re-run verify_setup and retry after mql_bridge.state is ready."
            })))
        }
    };
    if let Err(error) = response.require_ok() {
        return Ok(tool_error(json!({
            "code": response.get("code").unwrap_or("symbol_select_failed"),
            "error": error.to_string(),
            "requested_symbol": requested,
            "resolved_symbol": resolved,
            "symbol_resolution": resolution,
            "active_server": health.account_server,
            "mt5_error": response.get("mt5_error"),
            "hint": "Confirm the symbol is available for this broker account and retry."
        })));
    }

    let synchronized = response.bool("synchronized");
    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "request_id": response.get("request_id"),
            "requested_symbol": requested,
            "resolved_symbol": resolved,
            "symbol_resolution": resolution,
            "already_selected": response.bool("already_selected"),
            "selected": response.bool("selected"),
            "visible": response.bool("visible"),
            "synchronized": synchronized,
            "warnings": if synchronized {
                Vec::<Value>::new()
            } else {
                vec![json!({
                    "code": "symbol_not_yet_synchronized",
                    "message": "Symbol is selected and visible, but price/history synchronization is still pending."
                })]
            },
            "active_account": {
                "server": health.account_server,
                "login": health.account_login,
            },
            "bridge": {
                "state": health.state,
                "protocol_version": health.protocol_version,
                "service_version": health.service_version,
                "terminal_instance_id": bridge.instance_id(),
                "terminal_build": health.terminal_build,
                "heartbeat_age_seconds": health.heartbeat_age_seconds,
            }
        }).to_string() }],
        "isError": false
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_timeout_before_touching_bridge() {
        let response = handle_ensure_market_watch_symbol(
            &Config::default(),
            &json!({ "symbol": "EURUSD", "timeout_ms": 10 }),
        )
        .await
        .unwrap();
        let body: Value =
            serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["code"], "invalid_timeout");
    }
}
