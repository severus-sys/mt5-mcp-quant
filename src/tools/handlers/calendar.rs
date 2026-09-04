use crate::bridge::{validate_id, BridgeClient, BridgeHealthState};
use crate::compile::MqlCompiler;
use crate::models::Config;
use crate::utils::atomic_write;
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const PROVIDER_SOURCE: &str = include_str!("../../../mql/CalendarStaticProvider.mqh");
const CSV_SCHEMA_VERSION: &str = "1";
static CALENDAR_STATE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
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
        let bridge = BridgeClient::new(config).ok();
        let can_start = existing.state == CalendarJobState::Prepared
            && bridge
                .as_ref()
                .map(|value| {
                    let health = value.health();
                    health.state == BridgeHealthState::Ready && health.connected != Some(false)
                })
                .unwrap_or(false);
        if can_start {
            existing.state = CalendarJobState::Running;
            existing.progress_percent = 1;
            existing.terminal_instance_id =
                bridge.as_ref().map(|value| value.instance_id().to_string());
            existing.error_code = None;
            existing.error_message = None;
            existing.updated_at = chrono::Utc::now().to_rfc3339();
            write_job(&existing)?;
            let background_config = config.clone();
            let background_job_id = job_id.clone();
            tokio::spawn(async move {
                if let Err(error) = run_export_job(background_config, &background_job_id).await {
                    if let Ok(mut failed) = read_job(&background_job_id) {
                        failed.state = CalendarJobState::Failed;
                        failed.updated_at = chrono::Utc::now().to_rfc3339();
                        failed.error_code = Some("calendar_export_failed".into());
                        failed.error_message = Some(error.to_string());
                        let _ = write_job(&failed);
                    }
                }
            });
        }
        return Ok(success(json!({
            "success": true,
            "idempotent": true,
            "job": existing,
            "auto_started": can_start,
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
        let config = config.clone();
        let background_job_id = job_id.clone();
        tokio::spawn(async move {
            if let Err(error) = run_export_job(config, &background_job_id).await {
                if let Ok(mut failed) = read_job(&background_job_id) {
                    failed.state = CalendarJobState::Failed;
                    failed.updated_at = chrono::Utc::now().to_rfc3339();
                    failed.error_code = Some("calendar_export_failed".into());
                    failed.error_message = Some(error.to_string());
                    let _ = write_job(&failed);
                }
            }
        });
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
        .request("export_calendar", &fields, Duration::from_secs(7_200))
        .await?;
    response.require_ok()?;
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
            CalendarJobState::Validated | CalendarJobState::Partial
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

    if target.exists() {
        let backup = datasets_root.join(format!(".old-{}", uuid::Uuid::new_v4().simple()));
        fs::rename(&target, &backup)?;
        if let Err(error) = fs::rename(&staging, &target) {
            let _ = fs::rename(&backup, &target);
            return Err(error.into());
        }
        let _ = fs::remove_dir_all(backup);
    } else {
        fs::rename(&staging, &target)?;
    }
    MqlCompiler::new(config.clone())
        .deploy_include("CalendarStaticProvider.mqh", PROVIDER_SOURCE.as_bytes())?;
    job.state = CalendarJobState::Ready;
    if !job.datasets.iter().any(|value| value == dataset_id) {
        job.datasets.push(dataset_id.to_string());
    }
    job.updated_at = chrono::Utc::now().to_rfc3339();
    write_job(&job)?;

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
            "class": "CMt5MqCalendarStaticProvider",
            "load": format!("Load(\"{}\")", dataset_id),
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
}
