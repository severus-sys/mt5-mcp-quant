use crate::bridge::{validate_id, BridgeClient, BridgeHealthState, BridgeResponse};
use crate::compile::MqlCompiler;
use crate::models::Config;
use crate::utils::atomic_write;
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const PROVIDER_SOURCE: &str = include_str!("../../../mql/CalendarStaticProvider.mqh");
const CSV_SCHEMA_VERSION: &str = "1";
static CALENDAR_STATE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static ACTIVE_EXPORTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
const CSV_HEADER: [&str; 28] = [
    "schema_version",
    "value_id",
    "event_id",
    "time_server_epoch",
    "time_server",
    "period_server_epoch",
    "period_server",
    "revision",
    "country_id",
    "country_code",
    "country_name",
    "currency",
    "event_type",
    "sector",
    "frequency",
    "time_mode",
    "unit",
    "importance",
    "multiplier",
    "digits",
    "event_code",
    "event_name",
    "source_url",
    "impact_type",
    "actual",
    "previous",
    "revised_previous",
    "forecast",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CalendarJobState {
    Prepared,
    Running,
    Complete,
    Partial,
    Failed,
    Validated,
    Invalid,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalendarFilters {
    currencies: Vec<String>,
    country_codes: Vec<String>,
    importance: Vec<String>,
    from: String,
    to: String,
    from_server_epoch: i64,
    to_server_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalendarCoverage {
    requested_from: String,
    requested_to: String,
    observed_from: Option<String>,
    observed_to: Option<String>,
    completeness: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalendarJob {
    schema_version: u32,
    job_id: String,
    fingerprint: String,
    state: CalendarJobState,
    created_at: String,
    updated_at: String,
    filters: CalendarFilters,
    broker_server: Option<String>,
    account_login: Option<String>,
    terminal_instance_id: Option<String>,
    terminal_build: Option<String>,
    progress_percent: u32,
    row_count: u64,
    export_path: Option<String>,
    export_sha256: Option<String>,
    validation_valid: bool,
    coverage: CalendarCoverage,
    error_code: Option<String>,
    error_message: Option<String>,
    datasets: Vec<String>,
}

fn tool_error(body: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": body.to_string() }],
        "isError": true
    })
}

fn success(body: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": body.to_string() }],
        "isError": false
    })
}

fn jobs_root() -> PathBuf {
    Config::installation_dir().join("calendar").join("jobs")
}

fn job_path(job_id: &str) -> PathBuf {
    jobs_root().join(job_id).join("job.json")
}

fn write_job(job: &CalendarJob) -> Result<()> {
    atomic_write(&job_path(&job.job_id), &serde_json::to_vec_pretty(job)?)
}

fn read_job(job_id: &str) -> Result<CalendarJob> {
    validate_id(job_id)?;
    let path = job_path(job_id);
    serde_json::from_slice(&fs::read(&path).with_context(|| format!("job not found: {}", job_id))?)
        .context("calendar job metadata is invalid")
}

fn normalized_list(args: &Value, key: &str, length: usize) -> Result<Vec<String>> {
    let mut values = args
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|value| value.len() == length)
                        .filter(|value| {
                            value
                                .chars()
                                .all(|character| character.is_ascii_alphabetic())
                        })
                        .map(str::to_ascii_uppercase)
                        .ok_or_else(|| anyhow!("{} contains an invalid value", key))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    values.sort();
    values.dedup();
    Ok(values)
}

fn normalize_filters(args: &Value) -> Result<CalendarFilters> {
    let currencies = normalized_list(args, "currencies", 3)?;
    let country_codes = normalized_list(args, "country_codes", 2)?;
    if currencies.is_empty() && country_codes.is_empty() {
        bail!("at least one currency or country_code is required");
    }
    let mut importance = args
        .get("importance")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let value = item
                        .as_str()
                        .map(str::to_ascii_lowercase)
                        .ok_or_else(|| anyhow!("importance contains a non-string value"))?;
                    if !["low", "moderate", "high"].contains(&value.as_str()) {
                        bail!("invalid importance: {}", value);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec!["high".to_string()]);
    importance.sort();
    importance.dedup();
    if importance.is_empty() {
        bail!("importance cannot be empty");
    }
    let from = args
        .get("from")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("from is required"))?;
    let to = args
        .get("to")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("to is required"))?;
    if from.ends_with('Z') || to.ends_with('Z') || from.contains('+') || to.contains('+') {
        bail!(
            "calendar dates are broker server-time values and must not include a timezone suffix"
        );
    }
    let parse = |value: &str| {
        NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
            .map_err(|_| anyhow!("date must use YYYY-MM-DDTHH:MM:SS broker server-time format"))
    };
    let from_time = parse(from)?;
    let to_time = parse(to)?;
    if to_time <= from_time {
        bail!("to must be later than from");
    }
    Ok(CalendarFilters {
        currencies,
        country_codes,
        importance,
        from: from.to_string(),
        to: to.to_string(),
        from_server_epoch: from_time.and_utc().timestamp(),
        to_server_epoch: to_time.and_utc().timestamp(),
    })
}

fn fingerprint(
    filters: &CalendarFilters,
    broker_server: Option<&str>,
    terminal_instance_id: Option<&str>,
) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!({
            "filters": filters,
            "broker_server": broker_server,
            "terminal_instance_id": terminal_instance_id,
        }))?)
    ))
}

fn export_is_active(job_id: &str) -> bool {
    ACTIVE_EXPORTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(job_id)
}

fn mark_export_failed(job_id: &str, error: &anyhow::Error) {
    if let Ok(mut failed) = read_job(job_id) {
        if failed.state != CalendarJobState::Running {
            return;
        }
        failed.state = CalendarJobState::Failed;
        failed.updated_at = chrono::Utc::now().to_rfc3339();
        failed.error_code = Some("calendar_export_failed".into());
        failed.error_message = Some(error.to_string());
        let _ = write_job(&failed);
    }
}

fn spawn_export_job(config: Config, job_id: String) {
    ACTIVE_EXPORTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(job_id.clone());
    tokio::spawn(async move {
        let result = run_export_job(config, &job_id).await;
        ACTIVE_EXPORTS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&job_id);
        if let Err(error) = result {
            mark_export_failed(&job_id, &error);
        }
    });
}

pub async fn handle_prepare_calendar_export(config: &Config, args: &Value) -> Result<Value> {
    let _state_guard = CALENDAR_STATE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    if args
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or("csv")
        != "csv"
    {
        return Ok(tool_error(json!({
            "code": "unsupported_output_format",
            "error": "Only CSV schema v1 is supported."
        })));
    }
    let filters = match normalize_filters(args) {
        Ok(filters) => filters,
        Err(error) => {
            return Ok(tool_error(json!({
                "code": "invalid_calendar_filter",
                "error": error.to_string(),
                "time_semantics": "broker_server_time_without_timezone"
            })))
        }
    };
    let account = config.current_account();
    let bridge_identity = BridgeClient::new(config).ok();
    let fingerprint = fingerprint(
        &filters,
        account.as_ref().map(|value| value.server.as_str()),
        bridge_identity.as_ref().map(BridgeClient::instance_id),
    )?;
    let job_id = format!("cal_{}", &fingerprint[..24]);
    let overwrite = args
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if job_path(&job_id).is_file() && !overwrite {
        let mut existing = read_job(&job_id)?;
        if existing.state == CalendarJobState::Running && !export_is_active(&job_id) {
            if let Err(error) = reconcile_running_job(config, &job_id) {
                mark_export_failed(&job_id, &error);
            }
            existing = read_job(&job_id)?;
        }

        let mut can_start = false;
        let mut installation = None;
        let mut provider_path = None;
        let mut bridge_health = None;
        let mut bridge_instance_id = None;
        if existing.state == CalendarJobState::Prepared {
            let bridge = match BridgeClient::new(config) {
                Ok(bridge) => bridge,
                Err(error) => {
                    existing.error_code = Some("bridge_not_configured".into());
                    existing.error_message = Some(error.to_string());
                    existing.updated_at = chrono::Utc::now().to_rfc3339();
                    write_job(&existing)?;
                    return Ok(success(json!({
                        "success": true,
                        "idempotent": true,
                        "job": existing,
                        "auto_started": false,
                        "inspect_with": { "tool": "inspect_calendar_export", "job_id": job_id }
                    })));
                }
            };
            bridge_instance_id = Some(bridge.instance_id().to_string());
            installation = match bridge.ensure_installed().await {
                Ok(result) => Some(result),
                Err(error) => {
                    existing.error_code = Some("bridge_install_failed".into());
                    existing.error_message = Some(error.to_string());
                    existing.updated_at = chrono::Utc::now().to_rfc3339();
                    write_job(&existing)?;
                    return Ok(success(json!({
                        "success": true,
                        "idempotent": true,
                        "job": existing,
                        "auto_started": false,
                        "start_service_once": "Fix MetaEditor compilation errors, then retry.",
                        "inspect_with": { "tool": "inspect_calendar_export", "job_id": job_id }
                    })));
                }
            };
            provider_path = match MqlCompiler::new(config.clone())
                .deploy_include("CalendarStaticProvider.mqh", PROVIDER_SOURCE.as_bytes())
            {
                Ok(path) => Some(path),
                Err(error) => {
                    existing.error_code = Some("calendar_provider_install_failed".into());
                    existing.error_message = Some(error.to_string());
                    existing.updated_at = chrono::Utc::now().to_rfc3339();
                    write_job(&existing)?;
                    return Ok(success(json!({
                        "success": true,
                        "idempotent": true,
                        "job": existing,
                        "auto_started": false,
                        "installation": installation,
                        "inspect_with": { "tool": "inspect_calendar_export", "job_id": job_id }
                    })));
                }
            };
            let health = bridge.health();
            can_start = health.state == BridgeHealthState::Ready && health.connected != Some(false);
            bridge_health = Some(health);
            existing.error_code = None;
            existing.error_message = None;
            existing.updated_at = chrono::Utc::now().to_rfc3339();
        }
        if can_start {
            existing.state = CalendarJobState::Running;
            existing.progress_percent = 1;
            existing.terminal_instance_id = bridge_instance_id;
        }
        write_job(&existing)?;
        if can_start {
            spawn_export_job(config.clone(), job_id.clone());
        }
        return Ok(success(json!({
            "success": true,
            "idempotent": true,
            "job": existing,
            "auto_started": can_start,
            "installation": installation,
            "provider_path": provider_path,
            "bridge": bridge_health,
            "inspect_with": { "tool": "inspect_calendar_export", "job_id": job_id }
        })));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut job = CalendarJob {
        schema_version: 1,
        job_id: job_id.clone(),
        fingerprint,
        state: CalendarJobState::Prepared,
        created_at: now.clone(),
        updated_at: now,
        broker_server: account.as_ref().map(|value| value.server.clone()),
        account_login: account.as_ref().map(|value| value.login.clone()),
        terminal_instance_id: None,
        terminal_build: None,
        progress_percent: 0,
        row_count: 0,
        export_path: None,
        export_sha256: None,
        validation_valid: false,
        coverage: CalendarCoverage {
            requested_from: filters.from.clone(),
            requested_to: filters.to.clone(),
            observed_from: None,
            observed_to: None,
            completeness: "unknown".to_string(),
        },
        filters,
        error_code: None,
        error_message: None,
        datasets: Vec::new(),
    };
    write_job(&job)?;

    let bridge = match BridgeClient::new(config) {
        Ok(bridge) => bridge,
        Err(error) => {
            job.error_code = Some("bridge_not_configured".into());
            job.error_message = Some(error.to_string());
            write_job(&job)?;
            return Ok(success(json!({
                "success": true,
                "job": job,
                "auto_started": false,
                "start_service_once": "Run scripts/setup.ps1, then inspect verify_setup.mql_bridge."
            })));
        }
    };
    let installation = match bridge.ensure_installed().await {
        Ok(result) => result,
        Err(error) => {
            job.error_code = Some("bridge_install_failed".into());
            job.error_message = Some(error.to_string());
            write_job(&job)?;
            return Ok(success(json!({
                "success": true,
                "job": job,
                "auto_started": false,
                "start_service_once": "Fix MetaEditor compilation errors, then retry."
            })));
        }
    };
    let provider_path = MqlCompiler::new(config.clone())
        .deploy_include("CalendarStaticProvider.mqh", PROVIDER_SOURCE.as_bytes())?;
    let health = bridge.health();
    if health.state == BridgeHealthState::Ready && health.connected != Some(false) {
        job.state = CalendarJobState::Running;
        job.progress_percent = 1;
        job.terminal_instance_id = Some(bridge.instance_id().to_string());
        job.updated_at = chrono::Utc::now().to_rfc3339();
        write_job(&job)?;
        spawn_export_job(config.clone(), job_id.clone());
        Ok(success(json!({
            "success": true,
            "job": job,
            "auto_started": true,
            "installation": installation,
            "provider_path": provider_path,
            "inspect_with": { "tool": "inspect_calendar_export", "job_id": job_id }
        })))
    } else {
        Ok(success(json!({
            "success": true,
            "job": job,
            "auto_started": false,
            "bridge": health,
            "installation": installation,
            "provider_path": provider_path,
            "start_service_once": [
                "Open MT5 Navigator (Ctrl+N).",
                "Expand Services > MT5-MCP-Quant.",
                "Start MT5McpQuantBridge once.",
                "Call prepare_calendar_export again; the fingerprint makes it idempotent."
            ]
        })))
    }
}

async fn run_export_job(config: Config, job_id: &str) -> Result<()> {
    let mut job = read_job(job_id)?;
    job.state = CalendarJobState::Running;
    job.progress_percent = 1;
    job.updated_at = chrono::Utc::now().to_rfc3339();
    write_job(&job)?;
    let bridge = BridgeClient::new(&config)?;
    let fields = BTreeMap::from([
        ("job_id".into(), job.job_id.clone()),
        ("fingerprint".into(), job.fingerprint.clone()),
        ("currencies".into(), job.filters.currencies.join(",")),
        ("countries".into(), job.filters.country_codes.join(",")),
        ("importance".into(), job.filters.importance.join(",")),
        (
            "from_epoch".into(),
            job.filters.from_server_epoch.to_string(),
        ),
        ("to_epoch".into(), job.filters.to_server_epoch.to_string()),
    ]);
    let response = bridge
        .request_with_id(
            "export_calendar",
            &fields,
            Duration::from_secs(7_200),
            job_id,
        )
        .await?;
    finalize_export_response(&config, job_id, response)
}

fn finalize_export_response(config: &Config, job_id: &str, response: BridgeResponse) -> Result<()> {
    response.require_ok()?;
    let mut job = read_job(job_id)?;
    job.state = CalendarJobState::Complete;
    job.progress_percent = 95;
    job.terminal_instance_id = response.get("instance_id").map(str::to_string);
    job.terminal_build = response.get("terminal_build").map(str::to_string);
    job.broker_server = response.get("account_server").map(str::to_string);
    job.updated_at = chrono::Utc::now().to_rfc3339();
    write_job(&job)?;

    let common_files = config
        .terminal_common_files_dir()
        .ok_or_else(|| anyhow!("terminal_common_data_dir not configured"))?;
    let raw_relative = response
        .get("raw_file")
        .ok_or_else(|| anyhow!("bridge response omitted raw_file"))?;
    if raw_relative.contains("..") || Path::new(raw_relative).is_absolute() {
        bail!("bridge returned an unsafe raw_file path");
    }
    let raw_path = common_files.join(raw_relative.replace('/', "\\"));
    let canonical_path = jobs_root().join(job_id).join("export.csv");
    let validation = match validate_export(&raw_path, &canonical_path) {
        Ok(validation) => validation,
        Err(error) => {
            job.state = CalendarJobState::Invalid;
            job.validation_valid = false;
            job.error_code = Some("calendar_export_invalid".into());
            job.error_message = Some(error.to_string());
            job.updated_at = chrono::Utc::now().to_rfc3339();
            write_job(&job)?;
            return Ok(());
        }
    };
    job.row_count = validation.row_count;
    job.export_path = Some(canonical_path.to_string_lossy().into_owned());
    job.export_sha256 = Some(validation.sha256);
    job.validation_valid = true;
    job.progress_percent = 100;
    job.coverage.observed_from = validation.observed_from;
    job.coverage.observed_to = validation.observed_to;
    job.coverage.completeness = response
        .get("completeness")
        .unwrap_or("complete")
        .to_string();
    job.state = if job.coverage.completeness == "complete" {
        CalendarJobState::Validated
    } else {
        CalendarJobState::Partial
    };
    job.updated_at = chrono::Utc::now().to_rfc3339();
    write_job(&job)
}

fn reconcile_running_job(config: &Config, job_id: &str) -> Result<bool> {
    if export_is_active(job_id) {
        return Ok(false);
    }
    let bridge = BridgeClient::new(config)?;
    let response = match bridge.take_response(job_id)? {
        Some(response) => Some(response),
        None => bridge.take_calendar_response(job_id)?,
    };
    let Some(response) = response else {
        return Ok(false);
    };
    if let Err(error) = finalize_export_response(config, job_id, response) {
        mark_export_failed(job_id, &error);
        return Err(error);
    }
    Ok(true)
}

struct ValidationResult {
    row_count: u64,
    sha256: String,
    observed_from: Option<String>,
    observed_to: Option<String>,
}

fn validate_export(source: &Path, destination: &Path) -> Result<ValidationResult> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_path(source)
        .with_context(|| format!("open raw calendar CSV: {}", source.display()))?;
    if reader.headers()?.iter().collect::<Vec<_>>() != CSV_HEADER.as_slice() {
        bail!("calendar CSV schema/header mismatch");
    }
    let mut rows_by_value_id = BTreeMap::new();
    for record in reader.records() {
        let record = record?;
        if record.len() != CSV_HEADER.len() || record.get(0) != Some(CSV_SCHEMA_VERSION) {
            bail!("calendar CSV row has an invalid schema or column count");
        }
        let value_id = record[1].parse::<u64>().context("invalid value_id")?;
        record[2].parse::<u64>().context("invalid event_id")?;
        record[3]
            .parse::<i64>()
            .context("invalid time_server_epoch")?;
        for index in 24..=27 {
            if !record[index].is_empty() {
                record[index]
                    .parse::<i64>()
                    .with_context(|| format!("invalid raw int64 in {}", CSV_HEADER[index]))?;
            }
        }
        if let Some(existing) = rows_by_value_id.get(&value_id) {
            if existing != &record {
                bail!("conflicting duplicate calendar value_id: {}", value_id);
            }
            continue;
        }
        rows_by_value_id.insert(value_id, record);
    }
    let mut rows = rows_by_value_id.into_values().collect::<Vec<_>>();
    rows.sort_by_key(|record| {
        (
            record[3].parse::<i64>().unwrap_or_default(),
            record[2].parse::<u64>().unwrap_or_default(),
            record[1].parse::<u64>().unwrap_or_default(),
        )
    });
    let observed_from = rows.first().map(|record| record[4].to_string());
    let observed_to = rows.last().map(|record| record[4].to_string());
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    writer.write_record(CSV_HEADER)?;
    for row in &rows {
        writer.write_record(row)?;
    }
    let bytes = writer.into_inner()?;
    atomic_write(destination, &bytes)?;
    Ok(ValidationResult {
        row_count: rows.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        observed_from,
        observed_to,
    })
}

pub async fn handle_inspect_calendar_export(config: &Config, args: &Value) -> Result<Value> {
    let job_id = args
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("job_id is required"))?;
    let mut job = match read_job(job_id) {
        Ok(job) => job,
        Err(error) => {
            return Ok(tool_error(json!({
                "code": "calendar_job_not_found",
                "error": error.to_string(),
                "job_id": job_id,
            })))
        }
    };
    if job.state == CalendarJobState::Running && !export_is_active(job_id) {
        if let Err(error) = reconcile_running_job(config, job_id) {
            mark_export_failed(job_id, &error);
        }
        job = read_job(job_id)?;
    }
    if job.state == CalendarJobState::Running {
        if let Some(common) = config.terminal_common_files_dir() {
            let progress = common
                .join("mt5-mcp-quant")
                .join("calendar")
                .join("jobs")
                .join(job_id)
                .join("progress.kv");
            if let Ok(raw) = fs::read_to_string(progress) {
                for line in raw.lines() {
                    if let Some(value) = line.strip_prefix("progress_percent=") {
                        job.progress_percent = value.parse().unwrap_or(job.progress_percent);
                    } else if let Some(value) = line.strip_prefix("row_count=") {
                        job.row_count = value.parse().unwrap_or(job.row_count);
                    }
                }
            }
        }
    }
    let validate_rows = args
        .get("validate_rows")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if validate_rows && job.validation_valid {
        let current_checksum = job
            .export_path
            .as_deref()
            .and_then(|path| fs::read(path).ok())
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        if current_checksum.as_deref() != job.export_sha256.as_deref() {
            job.state = CalendarJobState::Invalid;
            job.validation_valid = false;
            job.error_code = Some("calendar_export_checksum_mismatch".into());
            job.error_message = Some("The validated export is missing or changed on disk.".into());
            job.updated_at = chrono::Utc::now().to_rfc3339();
            write_job(&job)?;
        }
    }
    Ok(success(json!({
        "success": true,
        "job": job,
        "polling": "idempotent",
        "time_semantics": "broker_server_time_without_timezone",
        "missing_numeric_values": "empty CSV field; zero remains 0",
    })))
}

pub async fn handle_prepare_calendar_backtest_dataset(
    config: &Config,
    args: &Value,
) -> Result<Value> {
    let _state_guard = CALENDAR_STATE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let job_id = args
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("job_id is required"))?;
    let dataset_id = args
        .get("dataset_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("dataset_name is required"))?;
    validate_id(job_id)?;
    if dataset_id.is_empty()
        || dataset_id.len() > 64
        || !dataset_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Ok(tool_error(json!({
            "code": "invalid_dataset_name",
            "error": "dataset_name may contain only letters, digits, dot, dash, and underscore."
        })));
    }
    let mut job = read_job(job_id)?;
    if !job.validation_valid
        || !matches!(
            job.state,
            CalendarJobState::Validated | CalendarJobState::Partial | CalendarJobState::Ready
        )
    {
        return Ok(tool_error(json!({
            "code": "calendar_export_not_validated",
            "error": "Only a structurally validated export can become a tester dataset.",
            "job": job,
        })));
    }
    let export_path = PathBuf::from(
        job.export_path
            .as_deref()
            .ok_or_else(|| anyhow!("validated job has no export_path"))?,
    );
    let bytes = fs::read(&export_path)?;
    let checksum = format!("{:x}", Sha256::digest(&bytes));
    if job.export_sha256.as_deref() != Some(checksum.as_str()) {
        return Ok(tool_error(json!({
            "code": "calendar_export_checksum_mismatch",
            "error": "The validated export changed after validation; dataset publication was refused."
        })));
    }
    let bridge = BridgeClient::new(config)?;
    if job.terminal_instance_id.as_deref() != Some(bridge.instance_id()) {
        return Ok(tool_error(json!({
            "code": "calendar_terminal_context_mismatch",
            "error": "The validated export belongs to a different MT5 terminal data instance.",
            "export_terminal_instance_id": job.terminal_instance_id,
            "current_terminal_instance_id": bridge.instance_id(),
        })));
    }
    if job.broker_server.as_deref().unwrap_or("").is_empty()
        || !matches!(job.coverage.completeness.as_str(), "complete" | "partial")
    {
        return Ok(tool_error(json!({
            "code": "calendar_export_context_incomplete",
            "error": "The validated export is missing broker identity or completeness metadata."
        })));
    }
    if let Some(current_account) = config.current_account() {
        if job.broker_server.as_deref() != Some(current_account.server.as_str()) {
            return Ok(tool_error(json!({
                "code": "calendar_broker_context_mismatch",
                "error": "The active broker server differs from the validated export.",
                "export_broker_server": job.broker_server,
                "active_broker_server": current_account.server,
            })));
        }
    }
    let common = config
        .terminal_common_files_dir()
        .ok_or_else(|| anyhow!("terminal_common_data_dir not configured"))?;
    let datasets_root = common
        .join("mt5-mcp-quant")
        .join("calendar")
        .join("datasets");
    fs::create_dir_all(&datasets_root)?;
    let target = datasets_root.join(dataset_id);
    let overwrite = args
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if target.exists() && !overwrite {
        return Ok(tool_error(json!({
            "code": "calendar_dataset_exists",
            "error": format!("Dataset '{}' already exists.", dataset_id),
            "hint": "Use a new dataset_name or pass overwrite=true explicitly."
        })));
    }
    let staging = datasets_root.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging)?;
    fs::write(staging.join("calendar.csv"), &bytes)?;
    let manifest = json!({
        "schema_version": 1,
        "dataset_id": dataset_id,
        "fingerprint": job.fingerprint,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "broker": {
            "server": job.broker_server,
            "account_login": job.account_login,
            "terminal_instance_id": job.terminal_instance_id,
            "terminal_build": job.terminal_build,
        },
        "time_semantics": "broker_server_time_without_timezone",
        "filters": job.filters,
        "coverage": job.coverage,
        "row_count": job.row_count,
        "bytes": bytes.len(),
        "sha256": checksum,
        "exporter_version": env!("CARGO_PKG_VERSION"),
        "source_job_id": job.job_id,
    });
    fs::write(
        staging.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let manifest_kv = format!(
        "schema_version=1\ndataset_id={}\ncsv_file=calendar.csv\ncsv_sha256={}\nbroker_server={}\nterminal_instance_id={}\ncompleteness={}\nrow_count={}\n",
        dataset_id,
        checksum,
        job.broker_server.as_deref().unwrap_or(""),
        bridge.instance_id(),
        job.coverage.completeness,
        job.row_count,
    );
    fs::write(staging.join("manifest.kv"), manifest_kv)?;

    let provider_path = match MqlCompiler::new(config.clone())
        .deploy_include("CalendarStaticProvider.mqh", PROVIDER_SOURCE.as_bytes())
    {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    let backup = if target.exists() {
        let backup = datasets_root.join(format!(".old-{}", uuid::Uuid::new_v4().simple()));
        fs::rename(&target, &backup)?;
        if let Err(error) = fs::rename(&staging, &target) {
            let _ = fs::rename(&backup, &target);
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        Some(backup)
    } else {
        if let Err(error) = fs::rename(&staging, &target) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        None
    };
    job.state = CalendarJobState::Ready;
    if !job.datasets.iter().any(|value| value == dataset_id) {
        job.datasets.push(dataset_id.to_string());
    }
    job.updated_at = chrono::Utc::now().to_rfc3339();
    if let Err(error) = write_job(&job) {
        let _ = fs::remove_dir_all(&target);
        if let Some(backup) = &backup {
            let _ = fs::rename(backup, &target);
        }
        return Err(error);
    }
    if let Some(backup) = backup {
        let _ = fs::remove_dir_all(backup);
    }

    Ok(success(json!({
        "success": true,
        "dataset_id": dataset_id,
        "dataset_dir": target,
        "csv_path": target.join("calendar.csv"),
        "manifest_path": target.join("manifest.json"),
        "sha256": checksum,
        "row_count": job.row_count,
        "completeness": job.coverage.completeness,
        "provider": {
            "include": "<MT5-MCP-Quant/CalendarStaticProvider.mqh>",
            "path": provider_path,
            "class": "CMt5MqCalendarStaticProvider",
            "load": format!("Load(\"{}\")", dataset_id),
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct StoredJobGuard {
        job_id: String,
    }

    impl Drop for StoredJobGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(jobs_root().join(&self.job_id));
        }
    }

    fn test_config(root: &Path) -> Config {
        let mut config = Config::default();
        config.data_dir = Some(root.join("terminal").to_string_lossy().into_owned());
        config.services_dir = Some(root.join("services").to_string_lossy().into_owned());
        config.include_dir = Some(root.join("include").to_string_lossy().into_owned());
        config.terminal_common_data_dir = Some(root.join("common").to_string_lossy().into_owned());
        config
    }

    fn valid_calendar_csv() -> Vec<u8> {
        let row = [
            "1",
            "10",
            "20",
            "1704067200",
            "2024-01-01T00:00:00",
            "0",
            "",
            "0",
            "840",
            "US",
            "United States",
            "USD",
            "CALENDAR_TYPE_INDICATOR",
            "CALENDAR_SECTOR_JOBS",
            "CALENDAR_FREQUENCY_MONTH",
            "CALENDAR_TIMEMODE_DATETIME",
            "CALENDAR_UNIT_PERCENT",
            "CALENDAR_IMPORTANCE_HIGH",
            "CALENDAR_MULTIPLIER_NONE",
            "1",
            "NFP",
            "Payrolls, total",
            "https://example.test",
            "CALENDAR_IMPACT_POSITIVE",
            "0",
            "",
            "-1000000",
            "2000000",
        ];
        let mut writer = csv::Writer::from_writer(Vec::new());
        writer.write_record(CSV_HEADER).unwrap();
        writer.write_record(row).unwrap();
        writer.into_inner().unwrap()
    }

    fn stored_job(
        root: &Path,
        config: &Config,
        job_id: String,
        state: CalendarJobState,
        validation_valid: bool,
    ) -> (StoredJobGuard, CalendarJob) {
        let export_path = root.join(format!("{}-export.csv", job_id));
        let bytes = valid_calendar_csv();
        fs::write(&export_path, &bytes).unwrap();
        let filters = normalize_filters(&json!({
            "currencies": ["USD"],
            "importance": ["high"],
            "from": "2024-01-01T00:00:00",
            "to": "2024-02-01T00:00:00"
        }))
        .unwrap();
        let bridge = BridgeClient::new(config).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let job = CalendarJob {
            schema_version: 1,
            job_id: job_id.clone(),
            fingerprint: "fixture-fingerprint".into(),
            state,
            created_at: now.clone(),
            updated_at: now,
            filters,
            broker_server: Some("Broker-Demo".into()),
            account_login: Some("123456".into()),
            terminal_instance_id: Some(bridge.instance_id().into()),
            terminal_build: Some("5000".into()),
            progress_percent: 50,
            row_count: 1,
            export_path: Some(export_path.to_string_lossy().into_owned()),
            export_sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
            validation_valid,
            coverage: CalendarCoverage {
                requested_from: "2024-01-01T00:00:00".into(),
                requested_to: "2024-02-01T00:00:00".into(),
                observed_from: Some("2024-01-01T00:00:00".into()),
                observed_to: Some("2024-01-01T00:00:00".into()),
                completeness: "complete".into(),
            },
            error_code: None,
            error_message: None,
            datasets: Vec::new(),
        };
        write_job(&job).unwrap();
        (StoredJobGuard { job_id }, job)
    }

    fn payload(response: &Value) -> Value {
        serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn normalization_is_idempotent_and_uses_or_within_categories() {
        let first = normalize_filters(&json!({
            "currencies": ["usd", "GBP", "USD"],
            "country_codes": ["us", "GB"],
            "importance": ["high", "low", "high"],
            "from": "2017-01-01T00:00:00",
            "to": "2026-12-31T23:59:59"
        }))
        .unwrap();
        assert_eq!(first.currencies, vec!["GBP", "USD"]);
        assert_eq!(first.country_codes, vec!["GB", "US"]);
        assert_eq!(first.importance, vec!["high", "low"]);
        assert_eq!(
            fingerprint(&first, Some("Broker-Demo"), Some("INSTANCE")).unwrap(),
            fingerprint(&first, Some("Broker-Demo"), Some("INSTANCE")).unwrap()
        );
        assert_ne!(
            fingerprint(&first, Some("Broker-Demo"), Some("INSTANCE")).unwrap(),
            fingerprint(&first, Some("Other-Demo"), Some("INSTANCE")).unwrap()
        );
    }

    #[test]
    fn normalization_rejects_wildcard_both_and_timezone_suffixes() {
        assert!(normalize_filters(&json!({
            "from": "2024-01-01T00:00:00",
            "to": "2024-02-01T00:00:00"
        }))
        .is_err());
        assert!(normalize_filters(&json!({
            "currencies": ["USD"],
            "from": "2024-01-01T00:00:00Z",
            "to": "2024-02-01T00:00:00Z"
        }))
        .is_err());
    }

    #[test]
    fn validation_preserves_zero_and_empty_and_merges_duplicate_ids() {
        let root = tempdir().unwrap();
        let source = root.path().join("raw.csv");
        let destination = root.path().join("validated.csv");
        let row = [
            "1",
            "10",
            "20",
            "1704067200",
            "2024-01-01T00:00:00",
            "0",
            "",
            "0",
            "840",
            "US",
            "United States",
            "USD",
            "CALENDAR_TYPE_INDICATOR",
            "CALENDAR_SECTOR_JOBS",
            "CALENDAR_FREQUENCY_MONTH",
            "CALENDAR_TIMEMODE_DATETIME",
            "CALENDAR_UNIT_PERCENT",
            "CALENDAR_IMPORTANCE_HIGH",
            "CALENDAR_MULTIPLIER_NONE",
            "1",
            "NFP",
            "Payrolls, total",
            "https://example.test",
            "CALENDAR_IMPACT_POSITIVE",
            "0",
            "",
            "-1000000",
            "2000000",
        ];
        let mut writer = csv::Writer::from_writer(Vec::new());
        writer.write_record(CSV_HEADER).unwrap();
        writer.write_record(row).unwrap();
        let valid = writer.into_inner().unwrap();
        fs::write(&source, &valid).unwrap();
        let result = validate_export(&source, &destination).unwrap();
        assert_eq!(result.row_count, 1);
        {
            let mut validated = csv::Reader::from_path(&destination).unwrap();
            let written = validated.records().next().unwrap().unwrap();
            assert_eq!(&written[24], "0");
            assert_eq!(&written[25], "");
            assert_eq!(&written[26], "-1000000");
            assert_eq!(&written[27], "2000000");
        }

        let mut duplicate = csv::Writer::from_writer(Vec::new());
        duplicate.write_record(CSV_HEADER).unwrap();
        duplicate.write_record(row).unwrap();
        duplicate.write_record(row).unwrap();
        fs::write(&source, duplicate.into_inner().unwrap()).unwrap();
        let deduplicated = validate_export(&source, &destination).unwrap();
        assert_eq!(deduplicated.row_count, 1);

        let mut conflicting = row;
        conflicting[24] = "1";
        let mut duplicate = csv::Writer::from_writer(Vec::new());
        duplicate.write_record(CSV_HEADER).unwrap();
        duplicate.write_record(row).unwrap();
        duplicate.write_record(conflicting).unwrap();
        fs::write(&source, duplicate.into_inner().unwrap()).unwrap();
        assert!(validate_export(&source, &destination).is_err());
    }

    #[tokio::test]
    async fn ready_export_can_publish_another_dataset() {
        let root = tempdir().unwrap();
        let config = test_config(root.path());
        let job_id = format!("test_ready_{}", uuid::Uuid::new_v4().simple());
        let (_guard, mut job) = stored_job(
            root.path(),
            &config,
            job_id.clone(),
            CalendarJobState::Ready,
            true,
        );
        job.datasets.push("first-dataset".into());
        write_job(&job).unwrap();

        let response = handle_prepare_calendar_backtest_dataset(
            &config,
            &json!({"job_id": job_id, "dataset_name": "second-dataset"}),
        )
        .await
        .unwrap();

        assert_eq!(response["isError"], false, "{}", payload(&response));
        assert_eq!(
            read_job(job.job_id.as_str()).unwrap().datasets,
            vec!["first-dataset", "second-dataset"]
        );
    }

    #[tokio::test]
    async fn provider_failure_does_not_publish_dataset_directory() {
        let root = tempdir().unwrap();
        let mut config = test_config(root.path());
        let blocked_include = root.path().join("include-is-a-file");
        fs::write(&blocked_include, b"not a directory").unwrap();
        config.include_dir = Some(blocked_include.to_string_lossy().into_owned());
        let job_id = format!("test_transaction_{}", uuid::Uuid::new_v4().simple());
        let (_guard, _job) = stored_job(
            root.path(),
            &config,
            job_id.clone(),
            CalendarJobState::Validated,
            true,
        );
        let dataset_id = "must-not-be-published";

        let result = handle_prepare_calendar_backtest_dataset(
            &config,
            &json!({"job_id": job_id, "dataset_name": dataset_id}),
        )
        .await;

        assert!(result.is_err());
        let target = config
            .terminal_common_files_dir()
            .unwrap()
            .join("mt5-mcp-quant")
            .join("calendar")
            .join("datasets")
            .join(dataset_id);
        assert!(
            !target.exists(),
            "provider failure must roll back the staged dataset"
        );
    }

    #[tokio::test]
    async fn prepared_retry_rechecks_bridge_installation() {
        let root = tempdir().unwrap();
        let config = test_config(root.path());
        let args = json!({
            "currencies": ["USD"],
            "importance": ["high"],
            "from": "2024-01-01T00:00:00",
            "to": "2024-02-01T00:00:00"
        });
        let filters = normalize_filters(&args).unwrap();
        let bridge = BridgeClient::new(&config).unwrap();
        let fingerprint = fingerprint(&filters, None, Some(bridge.instance_id())).unwrap();
        let job_id = format!("cal_{}", &fingerprint[..24]);
        let (_guard, mut job) = stored_job(
            root.path(),
            &config,
            job_id,
            CalendarJobState::Prepared,
            false,
        );
        job.fingerprint = fingerprint;
        job.error_code = Some("bridge_install_failed".into());
        job.error_message = Some("transient compiler failure".into());
        write_job(&job).unwrap();

        fs::create_dir_all(bridge.service_source_path().parent().unwrap()).unwrap();
        let source = include_bytes!("../../../mql/MT5McpQuantBridge.mq5");
        fs::write(bridge.service_source_path(), source).unwrap();
        fs::write(bridge.service_binary_path(), b"compiled fixture").unwrap();
        fs::write(
            bridge.service_source_path().with_extension("sha256"),
            format!("{:x}", Sha256::digest(source)),
        )
        .unwrap();

        let response = handle_prepare_calendar_export(&config, &args)
            .await
            .unwrap();
        let body = payload(&response);
        assert!(body.get("installation").is_some());
        assert!(body["job"]["error_code"].is_null());
    }

    #[tokio::test]
    async fn inspect_recovers_completed_running_job_after_restart() {
        let root = tempdir().unwrap();
        let config = test_config(root.path());
        let job_id = format!("test_recover_{}", uuid::Uuid::new_v4().simple());
        let (_guard, _job) = stored_job(
            root.path(),
            &config,
            job_id.clone(),
            CalendarJobState::Running,
            false,
        );
        let bridge = BridgeClient::new(&config).unwrap();
        fs::create_dir_all(bridge.root().join("requests")).unwrap();
        fs::create_dir_all(bridge.root().join("responses")).unwrap();
        let raw_relative = format!("mt5-mcp-quant/calendar/jobs/{}/raw.csv", job_id);
        let raw_path = config
            .terminal_common_files_dir()
            .unwrap()
            .join(raw_relative.replace('/', "\\"));
        fs::create_dir_all(raw_path.parent().unwrap()).unwrap();
        fs::write(&raw_path, valid_calendar_csv()).unwrap();
        let legacy_request_id = uuid::Uuid::new_v4().simple().to_string();
        let response_path = bridge
            .root()
            .join("responses")
            .join(format!("{}.res", legacy_request_id));
        fs::write(
            response_path,
            format!(
                "protocol=1\nrequest_id={}\ninstance_id={}\nok=true\nraw_file={}\naccount_server=Broker-Demo\nterminal_build=5000\ncompleteness=complete\n",
                legacy_request_id, bridge.instance_id(), raw_relative
            ),
        )
        .unwrap();

        let response = handle_inspect_calendar_export(&config, &json!({"job_id": job_id}))
            .await
            .unwrap();

        assert_eq!(payload(&response)["job"]["state"], "validated");
    }
}
