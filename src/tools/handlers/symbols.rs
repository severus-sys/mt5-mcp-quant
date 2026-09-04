use crate::bridge::{BridgeClient, BridgeHealthState};
use crate::models::{resolve_symbol, Config, SymbolMatch};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug)]
pub struct OperationalSymbol {
    pub requested: String,
    pub resolved: String,
    pub resolution: Value,
}

fn resolution_value(matched: &SymbolMatch) -> Value {
    match matched {
        SymbolMatch::Exact(_) => json!({ "status": "exact" }),
        SymbolMatch::Alias { kind, .. } => json!({ "status": "alias", "method": kind }),
        SymbolMatch::Ambiguous(candidates) => {
            json!({ "status": "ambiguous", "candidates": candidates })
        }
        SymbolMatch::NoMatch => json!({ "status": "no_match" }),
    }
}

pub async fn resolve_tester_symbol(
    config: &Config,
    supplied: &str,
    available: &[String],
    server: &str,
    login: &str,
) -> std::result::Result<OperationalSymbol, Value> {
    let effective = if supplied.trim().is_empty() {
        config.backtest_symbol.as_deref().unwrap_or("").trim()
    } else {
        supplied.trim()
    };

    if effective.is_empty() {
        if let Some(first) = available.first() {
            return Ok(OperationalSymbol {
                requested: String::new(),
                resolved: first.clone(),
                resolution: json!({ "status": "default", "method": "first_tester_symbol" }),
            });
        }
    }

    let tester_match = resolve_symbol(effective, available);
    let tester_resolution = resolution_value(&tester_match);
    if let Some(resolved) = tester_match.resolved() {
        return Ok(OperationalSymbol {
            requested: effective.to_string(),
            resolved: resolved.to_string(),
            resolution: tester_resolution,
        });
    }

    let mut code = if matches!(tester_match, SymbolMatch::Ambiguous(_)) {
        "symbol_ambiguous"
    } else {
        "symbol_not_in_tester_history"
    };
    let mut broker_status = "unknown";
    let mut candidates = tester_match.candidates().to_vec();
    let mut resolved_symbol = None;
    let mut resolution = tester_resolution;

    if matches!(tester_match, SymbolMatch::NoMatch) && !effective.is_empty() {
        if let Ok(bridge) = BridgeClient::new(config) {
            let health = bridge.health();
            if health.state == BridgeHealthState::Ready && health.connected != Some(false) {
                if let Ok(catalog) = bridge.list_server_symbols(Duration::from_secs(5)).await {
                    broker_status = "verified";
                    let broker_match = resolve_symbol(effective, &catalog);
                    resolution = resolution_value(&broker_match);
                    match &broker_match {
                        SymbolMatch::Exact(symbol)
                        | SymbolMatch::Alias {
                            resolved: symbol, ..
                        } => {
                            code = "symbol_history_not_found";
                            resolved_symbol = Some(symbol.clone());
                            candidates = vec![symbol.clone()];
                        }
                        SymbolMatch::Ambiguous(values) => {
                            code = "symbol_ambiguous";
                            candidates = values.clone();
                        }
                        SymbolMatch::NoMatch => code = "symbol_not_found",
                    }
                }
            }
        }
    }

    let hint = match code {
        "symbol_ambiguous" => {
            "Pass one exact symbol from candidates; no symbol was selected automatically."
        }
        "symbol_history_not_found" => {
            "The broker symbol exists, but Strategy Tester history is missing. Download history for the resolved symbol, then retry."
        }
        "symbol_not_found" => {
            "No matching symbol exists in the active broker catalog. Check the account/server or use the broker's exact symbol name."
        }
        _ => {
            "No matching local Strategy Tester history was found. Broker availability is unknown because the Service is not ready."
        }
    };

    Err(json!({
        "content": [{ "type": "text", "text": json!({
            "error": match code {
                "symbol_ambiguous" => format!("Symbol '{}' matches more than one candidate.", effective),
                "symbol_history_not_found" => format!("Symbol '{}' exists at the broker but has no local tester history.", effective),
                "symbol_not_found" => format!("Symbol '{}' does not exist in the active broker catalog.", effective),
                _ => format!("Symbol '{}' has no tester history on server '{}'.", effective, server),
            },
            "code": code,
            "pre_check": code,
            "requested_symbol": effective,
            "resolved_symbol": resolved_symbol,
            "symbol_resolution": resolution,
            "candidates": candidates,
            "available_tester_symbols": available,
            "active_server": server,
            "account": { "login": login, "server": server },
            "broker_status": broker_status,
            "hint": hint,
            "suggestion": hint,
        }).to_string() }],
        "isError": true
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{BRIDGE_PROTOCOL_VERSION, BRIDGE_SERVICE_VERSION};
    use crate::utils::atomic_write;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;

    fn payload(response: &Value) -> Value {
        serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn ambiguity_and_unknown_broker_are_safe() {
        let config = Config::default();
        let ambiguous = resolve_tester_symbol(
            &config,
            "EURUSD",
            &["EURUSDm".into(), "EURUSDz".into()],
            "Demo-Server",
            "123",
        )
        .await
        .unwrap_err();
        assert_eq!(payload(&ambiguous)["code"], "symbol_ambiguous");

        let missing =
            resolve_tester_symbol(&config, "EURUSD", &["GBPUSD".into()], "Demo-Server", "123")
                .await
                .unwrap_err();
        let body = payload(&missing);
        assert_eq!(body["code"], "symbol_not_in_tester_history");
        assert_eq!(body["broker_status"], "unknown");
    }

    #[tokio::test]
    async fn ready_broker_catalog_distinguishes_missing_tester_history() {
        let root = tempdir().unwrap();
        let data = root.path().join("terminal");
        let common = root.path().join("common");
        let mut config = Config::default();
        config.data_dir = Some(data.to_string_lossy().into_owned());
        config.services_dir = Some(data.join("MQL5/Services").to_string_lossy().into_owned());
        config.terminal_common_data_dir = Some(common.to_string_lossy().into_owned());
        let bridge = BridgeClient::new(&config).unwrap();
        fs::create_dir_all(bridge.service_source_path().parent().unwrap()).unwrap();
        fs::write(bridge.service_source_path(), b"fixture").unwrap();
        fs::write(bridge.service_binary_path(), b"fixture").unwrap();
        fs::create_dir_all(bridge.root()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(
            bridge.root().join("heartbeat.kv"),
            format!(
                "protocol={}\nservice_version={}\ninstance_id={}\nupdated_epoch={}\nconnected=true\n",
                BRIDGE_PROTOCOL_VERSION,
                BRIDGE_SERVICE_VERSION,
                bridge.instance_id(),
                now
            ),
        )
        .unwrap();

        let responder = bridge.clone();
        let task = tokio::spawn(async move {
            for _ in 0..100 {
                let requests = responder.root().join("requests");
                if let Some(path) = fs::read_dir(&requests)
                    .ok()
                    .and_then(|entries| entries.filter_map(|entry| entry.ok()).next())
                    .map(|entry| entry.path())
                {
                    let raw = fs::read_to_string(path).unwrap();
                    let request_id = raw
                        .lines()
                        .find_map(|line| line.strip_prefix("request_id="))
                        .unwrap();
                    let symbol_file = format!("{}.symbols", request_id);
                    let responses = responder.root().join("responses");
                    atomic_write(&responses.join(&symbol_file), b"EURUSDm\n").unwrap();
                    atomic_write(
                        &responses.join(format!("{}.res", request_id)),
                        format!(
                            "protocol={}\nrequest_id={}\ninstance_id={}\nok=true\nsymbols_file={}\n",
                            BRIDGE_PROTOCOL_VERSION,
                            request_id,
                            responder.instance_id(),
                            symbol_file
                        )
                        .as_bytes(),
                    )
                    .unwrap();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("broker catalog request was not published");
        });

        let result =
            resolve_tester_symbol(&config, "EURUSD", &["GBPUSD".into()], "Broker-Demo", "123")
                .await
                .unwrap_err();
        task.await.unwrap();
        let body = payload(&result);
        assert_eq!(body["code"], "symbol_history_not_found");
        assert_eq!(body["resolved_symbol"], "EURUSDm");
        assert_eq!(body["broker_status"], "verified");
    }
}
