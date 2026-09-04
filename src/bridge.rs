use crate::compile::{MqlCompiler, MqlTarget};
use crate::models::Config;
use crate::utils::atomic_write;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const BRIDGE_PROTOCOL_VERSION: &str = "1";
pub const BRIDGE_SERVICE_VERSION: &str = "1.0.1";
pub const BRIDGE_NAMESPACE: &str = "MT5-MCP-Quant";
const SERVICE_SOURCE: &str = include_str!("../mql/MT5McpQuantBridge.mq5");
static BRIDGE_INSTALL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeHealthState {
    NotInstalled,
    InstalledNotRunning,
    Ready,
    Stale,
    ProtocolMismatch,
    WrongTerminalInstance,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeHealth {
    pub state: BridgeHealthState,
    pub instance_id: String,
    pub service_source: String,
    pub service_binary: String,
    pub protocol_version: String,
    pub service_version: String,
    pub heartbeat_age_seconds: Option<u64>,
    pub terminal_data_path: Option<String>,
    pub account_login: Option<String>,
    pub account_server: Option<String>,
    pub terminal_build: Option<String>,
    pub connected: Option<bool>,
    pub hint: String,
}

#[derive(Debug, Clone)]
pub struct BridgeResponse {
    fields: BTreeMap<String, String>,
}

impl BridgeResponse {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key)
            .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
            .unwrap_or(false)
    }

    pub fn require_ok(&self) -> Result<()> {
        if self.bool("ok") {
            return Ok(());
        }
        bail!(
            "bridge request failed [{}]: {}",
            self.get("code").unwrap_or("bridge_error"),
            self.get("message").unwrap_or("unknown bridge error")
        )
    }
}

#[derive(Debug, Clone)]
pub struct BridgeClient {
    config: Config,
    instance_id: String,
    root: PathBuf,
}

impl BridgeClient {
    pub fn new(config: &Config) -> Result<Self> {
        let data_dir = config
            .mt5_dir()
            .ok_or_else(|| anyhow!("data_dir is not configured"))?;
        let common_files = config
            .terminal_common_files_dir()
            .ok_or_else(|| anyhow!("terminal_common_data_dir could not be discovered"))?;
        let instance_id = terminal_instance_id(&data_dir);
        let root = common_files
            .join("mt5-mcp-quant")
            .join("bridge")
            .join(format!("v{}", BRIDGE_PROTOCOL_VERSION))
            .join(&instance_id);
        Ok(Self {
            config: config.clone(),
            instance_id,
            root,
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn service_source_path(&self) -> PathBuf {
        self.config
            .services_dir()
            .join(BRIDGE_NAMESPACE)
            .join("MT5McpQuantBridge.mq5")
    }

    pub fn service_binary_path(&self) -> PathBuf {
        self.service_source_path().with_extension("ex5")
    }

    pub async fn ensure_installed(&self) -> Result<BridgeInstallResult> {
        let _install_guard = BRIDGE_INSTALL_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let expected_hash = format!("{:x}", Sha256::digest(SERVICE_SOURCE.as_bytes()));
        let hash_path = self.service_source_path().with_extension("sha256");
        let source_is_current = fs::read(self.service_source_path())
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)) == expected_hash)
            .unwrap_or(false);
        let already_current = source_is_current
            && self.service_binary_path().is_file()
            && fs::read_to_string(&hash_path)
                .map(|value| value.trim() == expected_hash)
                .unwrap_or(false);
        if already_current {
            return Ok(BridgeInstallResult {
                changed: false,
                source_path: self.service_source_path(),
                binary_path: self.service_binary_path(),
                source_sha256: expected_hash,
                warnings: Vec::new(),
            });
        }

        let stage_dir = std::env::temp_dir()
            .join("mt5-mcp-quant")
            .join("embedded-mql")
            .join(&self.instance_id);
        fs::create_dir_all(&stage_dir)?;
        let staged_source = stage_dir.join("MT5McpQuantBridge.mq5");
        atomic_write(&staged_source, SERVICE_SOURCE.as_bytes())?;

        let compiler = MqlCompiler::new(self.config.clone());
        let compiled = compiler
            .compile_target(
                &staged_source,
                MqlTarget::Service,
                BRIDGE_NAMESPACE,
                Duration::from_secs(120),
            )
            .await?;
        if !compiled.success {
            bail!(
                "MQL5 Service compilation failed: {}",
                compiled.errors.join("; ")
            );
        }
        atomic_write(&hash_path, format!("{}\n", expected_hash).as_bytes())?;
        self.ensure_protocol_dirs()?;
        Ok(BridgeInstallResult {
            changed: true,
            source_path: self.service_source_path(),
            binary_path: compiled
                .ex5_path
                .unwrap_or_else(|| self.service_binary_path()),
            source_sha256: expected_hash,
            warnings: compiled.warnings,
        })
    }

    pub fn health(&self) -> BridgeHealth {
        let base = |state, hint: &str| BridgeHealth {
            state,
            instance_id: self.instance_id.clone(),
            service_source: self.service_source_path().to_string_lossy().into_owned(),
            service_binary: self.service_binary_path().to_string_lossy().into_owned(),
            protocol_version: BRIDGE_PROTOCOL_VERSION.to_string(),
            service_version: BRIDGE_SERVICE_VERSION.to_string(),
            heartbeat_age_seconds: None,
            terminal_data_path: None,
            account_login: None,
            account_server: None,
            terminal_build: None,
            connected: None,
            hint: hint.to_string(),
        };

        if !self.service_source_path().is_file() || !self.service_binary_path().is_file() {
            return base(
                BridgeHealthState::NotInstalled,
                "Call a bridge-backed tool once to install and compile the embedded Service.",
            );
        }
        let heartbeat_path = self.root.join("heartbeat.kv");
        let Ok(raw) = fs::read_to_string(&heartbeat_path) else {
            return base(
                BridgeHealthState::InstalledNotRunning,
                "Start MT5McpQuantBridge once from Navigator > Services.",
            );
        };
        let Ok(fields) = parse_fields(&raw) else {
            return base(
                BridgeHealthState::InstalledNotRunning,
                "Heartbeat is malformed; restart the MT5McpQuantBridge Service.",
            );
        };
        let mut health = base(BridgeHealthState::Ready, "Bridge is ready.");
        health.terminal_data_path = fields.get("data_path").cloned();
        health.account_login = fields.get("account_login").cloned();
        health.account_server = fields.get("account_server").cloned();
        health.terminal_build = fields.get("terminal_build").cloned();
        health.connected = fields
            .get("connected")
            .map(|value| value.eq_ignore_ascii_case("true") || value == "1");
        let updated = fields
            .get("updated_epoch")
            .and_then(|value| value.parse::<u64>().ok());
        let now = now_epoch();
        let heartbeat_is_future = updated
            .map(|value| value > now.saturating_add(5))
            .unwrap_or(false);
        health.heartbeat_age_seconds = updated.map(|value| now.abs_diff(value));

        if fields.get("protocol").map(String::as_str) != Some(BRIDGE_PROTOCOL_VERSION)
            || fields.get("service_version").map(String::as_str) != Some(BRIDGE_SERVICE_VERSION)
        {
            health.state = BridgeHealthState::ProtocolMismatch;
            health.hint = "Reinstall the embedded Service so its protocol matches the MCP server."
                .to_string();
        } else if fields.get("instance_id").map(String::as_str) != Some(self.instance_id.as_str()) {
            health.state = BridgeHealthState::WrongTerminalInstance;
            health.hint =
                "Start the Service in the terminal instance configured by data_dir.".to_string();
        } else if let Some(account) = self.config.current_account() {
            if fields.get("account_login") != Some(&account.login)
                || fields.get("account_server") != Some(&account.server)
            {
                health.state = BridgeHealthState::WrongTerminalInstance;
                health.hint =
                    "Configured account/server differs from the Service heartbeat.".to_string();
            }
        }
        if health.state == BridgeHealthState::Ready && heartbeat_is_future {
            health.state = BridgeHealthState::Stale;
            health.hint = "Heartbeat timestamp is ahead of UTC; reinstall and restart the Service."
                .to_string();
        } else if health.state == BridgeHealthState::Ready
            && health.heartbeat_age_seconds.unwrap_or(u64::MAX) > 5
        {
            health.state = BridgeHealthState::Stale;
            health.hint = "Heartbeat is older than five seconds; restart the Service.".to_string();
        }
        health
    }

    pub async fn request(
        &self,
        operation: &str,
        fields: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<BridgeResponse> {
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        self.request_with_id(operation, fields, timeout, &request_id)
            .await
    }

    pub async fn request_with_id(
        &self,
        operation: &str,
        fields: &BTreeMap<String, String>,
        timeout: Duration,
        request_id: &str,
    ) -> Result<BridgeResponse> {
        if ![
            "list_server_symbols",
            "ensure_selected_exact",
            "export_calendar",
        ]
        .contains(&operation)
        {
            bail!("bridge operation is not allowlisted: {}", operation);
        }
        self.ensure_protocol_dirs()?;
        validate_id(request_id)?;
        let request_path = self
            .root
            .join("requests")
            .join(format!("{}.req", request_id));
        let response_path = self
            .root
            .join("responses")
            .join(format!("{}.res", request_id));
        let _ = fs::remove_file(&request_path);
        let _ = fs::remove_file(&response_path);
        let mut request = BTreeMap::from([
            ("protocol".to_string(), BRIDGE_PROTOCOL_VERSION.to_string()),
            ("request_id".to_string(), request_id.to_string()),
            ("instance_id".to_string(), self.instance_id.clone()),
            ("operation".to_string(), operation.to_string()),
            ("created_epoch".to_string(), now_epoch().to_string()),
            ("created_epoch_ms".to_string(), now_epoch_ms().to_string()),
            (
                "expires_epoch".to_string(),
                request_expiry_epoch(timeout).to_string(),
            ),
        ]);
        for (key, value) in fields {
            validate_field(key, value)?;
            if request.contains_key(key) {
                bail!("bridge field '{}' is reserved", key);
            }
            request.insert(key.clone(), value.clone());
        }
        atomic_write(&request_path, serialize_fields(&request).as_bytes())?;

        let started = std::time::Instant::now();
        while started.elapsed() <= timeout {
            if let Some(response) = self.take_response(request_id)? {
                return Ok(response);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = fs::remove_file(&request_path);
        let _ = fs::remove_file(&response_path);
        bail!(
            "bridge request '{}' timed out after {} ms",
            request_id,
            timeout.as_millis()
        )
    }

    pub fn take_response(&self, request_id: &str) -> Result<Option<BridgeResponse>> {
        self.read_response(request_id, true)
    }

    pub fn take_response_matching<F>(&self, mut matches: F) -> Result<Option<BridgeResponse>>
    where
        F: FnMut(&BridgeResponse) -> bool,
    {
        let responses_dir = self.root.join("responses");
        let mut response_ids = fs::read_dir(&responses_dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        let path = entry.path();
                        (path.extension().and_then(|value| value.to_str()) == Some("res"))
                            .then(|| {
                                path.file_stem()
                                    .and_then(|value| value.to_str())
                                    .map(str::to_string)
                            })
                            .flatten()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        response_ids.sort();
        for request_id in response_ids {
            if validate_id(&request_id).is_err() {
                continue;
            }
            let Ok(Some(response)) = self.read_response(&request_id, false) else {
                continue;
            };
            if !matches(&response) {
                continue;
            }
            let path = responses_dir.join(format!("{}.res", request_id));
            let _ = fs::remove_file(path);
            return Ok(Some(response));
        }
        Ok(None)
    }

    fn read_response(&self, request_id: &str, remove: bool) -> Result<Option<BridgeResponse>> {
        validate_id(request_id)?;
        let response_path = self
            .root
            .join("responses")
            .join(format!("{}.res", request_id));
        if !response_path.is_file() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&response_path)
            .with_context(|| format!("read bridge response {}", response_path.display()))?;
        let parsed = parse_fields(&raw)?;
        if parsed.get("request_id").map(String::as_str) != Some(request_id)
            || parsed.get("instance_id") != Some(&self.instance_id)
            || parsed.get("protocol").map(String::as_str) != Some(BRIDGE_PROTOCOL_VERSION)
        {
            bail!("bridge response identity or protocol mismatch");
        }
        if remove {
            let _ = fs::remove_file(&response_path);
        }
        Ok(Some(BridgeResponse { fields: parsed }))
    }

    pub async fn list_server_symbols(&self, timeout: Duration) -> Result<Vec<String>> {
        let response = self
            .request("list_server_symbols", &BTreeMap::new(), timeout)
            .await?;
        response.require_ok()?;
        let mut symbols = if let Some(file_name) = response.get("symbols_file") {
            let request_id = response
                .get("request_id")
                .ok_or_else(|| anyhow!("symbol response omitted request_id"))?;
            let expected = format!("{}.symbols", request_id);
            if file_name != expected {
                bail!("symbol response returned an unsafe catalog file name");
            }
            let path = self.root.join("responses").join(file_name);
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("read broker symbol catalog {}", path.display()))?;
            let decoded = raw
                .lines()
                .filter(|value| !value.is_empty())
                .map(decode_value)
                .collect::<Result<Vec<_>>>()?;
            let _ = fs::remove_file(path);
            decoded
        } else {
            response
                .get("symbols")
                .unwrap_or("")
                .split('|')
                .filter(|value| !value.is_empty())
                .map(decode_value)
                .collect::<Result<Vec<_>>>()?
        };
        symbols.sort();
        symbols.dedup();
        Ok(symbols)
    }

    fn ensure_protocol_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.root.join("requests"))?;
        fs::create_dir_all(self.root.join("responses"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeInstallResult {
    pub changed: bool,
    pub source_path: PathBuf,
    pub binary_path: PathBuf,
    pub source_sha256: String,
    pub warnings: Vec<String>,
}

pub fn terminal_instance_id(data_dir: &Path) -> String {
    let normalized = data_dir
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .chars()
        .map(|character| character.to_lowercase().next().unwrap_or(character))
        .collect::<String>();
    let mut hash = 0xcbf29ce484222325u64;
    // MQL5 StringToLower performs a simple, one-code-point lowercase mapping,
    // then StringGetCharacter exposes UTF-16 code units. Mirror both steps so
    // case-equivalent Windows paths bind to the same terminal instance.
    for code_unit in normalized.encode_utf16() {
        hash ^= u64::from((code_unit & 0x00ff) as u8);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016X}", hash)
}

pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        bail!("invalid bridge id")
    }
    Ok(())
}

fn validate_field(key: &str, value: &str) -> Result<()> {
    validate_id(key)?;
    if value.contains('\r') || value.contains('\n') || value.len() > 16_384 {
        bail!("invalid bridge field '{}': unsafe or oversized value", key);
    }
    Ok(())
}

fn serialize_fields(fields: &BTreeMap<String, String>) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}\n", key, encode_value(value)))
        .collect()
}

fn parse_fields(raw: &str) -> Result<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow!("malformed bridge field"))?;
        validate_id(key)?;
        if fields
            .insert(key.to_string(), decode_value(value)?)
            .is_some()
        {
            bail!("duplicate bridge field: {}", key);
        }
    }
    Ok(fields)
}

fn encode_value(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace('=', "%3D")
        .replace('|', "%7C")
}

fn decode_value(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                bail!("malformed percent escape");
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])?;
            decoded.push(u8::from_str_radix(hex, 16).context("invalid percent escape")?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).context("bridge value is not UTF-8")
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn request_expiry_epoch(timeout: Duration) -> u64 {
    now_epoch()
        .saturating_add(timeout.as_secs())
        .saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn bridge_fixture(root: &Path) -> BridgeClient {
        let data = root.join("terminal-instance");
        let common = root.join("common");
        let mut config = Config::default();
        config.data_dir = Some(data.to_string_lossy().into_owned());
        config.terminal_common_data_dir = Some(common.to_string_lossy().into_owned());
        BridgeClient::new(&config).unwrap()
    }

    #[test]
    fn instance_id_is_stable_across_windows_path_forms() {
        assert_eq!(
            terminal_instance_id(Path::new(r"C:\Data\Terminal\ABC\")),
            terminal_instance_id(Path::new("c:/data/terminal/abc"))
        );
        assert_eq!(
            terminal_instance_id(Path::new(r"C:\Veri\Türkçe\Terminal")),
            terminal_instance_id(Path::new(r"c:/veri/türkçe/terminal/"))
        );
    }

    #[test]
    fn service_and_rust_fold_non_ascii_path_case_consistently() {
        assert_eq!(
            terminal_instance_id(Path::new(r"C:\Veri\ÜST\Terminal")),
            terminal_instance_id(Path::new(r"c:\veri\üst\terminal"))
        );
        assert!(SERVICE_SOURCE.contains("StringToLower(value)"));
        assert!(!SERVICE_SOURCE.contains("AsciiLowerPath"));
    }

    #[test]
    fn ids_block_path_traversal_and_oversized_values() {
        assert!(validate_id("job_A-12").is_ok());
        assert!(validate_id("../escape").is_err());
        assert!(validate_id(&"a".repeat(65)).is_err());
        assert!(validate_field("symbol", "EURUSD\noperation=trade").is_err());
        assert!(parse_fields("ok=true\nok=false\n").is_err());
    }

    #[test]
    fn field_protocol_round_trips_reserved_characters() {
        let fields = BTreeMap::from([("message".to_string(), "a=b|c%20".to_string())]);
        assert_eq!(parse_fields(&serialize_fields(&fields)).unwrap(), fields);
        assert!(request_expiry_epoch(Duration::from_secs(5)) >= now_epoch() + 5);
    }

    #[tokio::test]
    async fn request_rejects_reserved_field_overrides() {
        let root = tempdir().unwrap();
        let bridge = bridge_fixture(root.path());
        let fields = BTreeMap::from([("operation".to_string(), "trade".to_string())]);
        let error = bridge
            .request("list_server_symbols", &fields, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("reserved"));
    }

    #[test]
    fn health_detects_stale_and_wrong_instances() {
        let root = tempdir().unwrap();
        let data = root.path().join("terminal-instance");
        let common = root.path().join("common");
        let services = data.join("MQL5").join("Services");
        let bridge_dir = services.join(BRIDGE_NAMESPACE);
        fs::create_dir_all(&bridge_dir).unwrap();
        fs::write(bridge_dir.join("MT5McpQuantBridge.mq5"), SERVICE_SOURCE).unwrap();
        fs::write(bridge_dir.join("MT5McpQuantBridge.ex5"), b"fixture").unwrap();
        let mut config = Config::default();
        config.data_dir = Some(data.to_string_lossy().into_owned());
        config.services_dir = Some(services.to_string_lossy().into_owned());
        config.terminal_common_data_dir = Some(common.to_string_lossy().into_owned());
        let bridge = BridgeClient::new(&config).unwrap();
        fs::create_dir_all(bridge.root()).unwrap();
        let stale = BTreeMap::from([
            ("protocol".into(), BRIDGE_PROTOCOL_VERSION.into()),
            ("service_version".into(), BRIDGE_SERVICE_VERSION.into()),
            ("instance_id".into(), bridge.instance_id().into()),
            ("updated_epoch".into(), "1".into()),
        ]);
        fs::write(bridge.root().join("heartbeat.kv"), serialize_fields(&stale)).unwrap();
        assert_eq!(bridge.health().state, BridgeHealthState::Stale);

        let future = BTreeMap::from([
            ("protocol".into(), BRIDGE_PROTOCOL_VERSION.into()),
            ("service_version".into(), BRIDGE_SERVICE_VERSION.into()),
            ("instance_id".into(), bridge.instance_id().into()),
            (
                "updated_epoch".into(),
                now_epoch().saturating_add(3_600).to_string(),
            ),
        ]);
        fs::write(
            bridge.root().join("heartbeat.kv"),
            serialize_fields(&future),
        )
        .unwrap();
        let health = bridge.health();
        assert_eq!(health.state, BridgeHealthState::Stale);
        assert!(health.hint.contains("ahead of UTC"));

        let wrong = BTreeMap::from([
            ("protocol".into(), BRIDGE_PROTOCOL_VERSION.into()),
            ("service_version".into(), BRIDGE_SERVICE_VERSION.into()),
            ("instance_id".into(), "WRONG".into()),
            ("updated_epoch".into(), now_epoch().to_string()),
        ]);
        fs::write(bridge.root().join("heartbeat.kv"), serialize_fields(&wrong)).unwrap();
        assert_eq!(
            bridge.health().state,
            BridgeHealthState::WrongTerminalInstance
        );
    }

    #[tokio::test]
    async fn request_times_out_when_service_is_not_running() {
        let root = tempdir().unwrap();
        let bridge = bridge_fixture(root.path());
        let error = bridge
            .request(
                "list_server_symbols",
                &BTreeMap::new(),
                Duration::from_millis(20),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert_eq!(
            fs::read_dir(bridge.root().join("requests"))
                .unwrap()
                .count(),
            0,
            "a timed-out operation must not remain executable in the Service queue"
        );
    }

    #[tokio::test]
    async fn installation_does_not_trust_binary_and_hash_without_source() {
        let root = tempdir().unwrap();
        let bridge = bridge_fixture(root.path());
        fs::create_dir_all(bridge.service_source_path().parent().unwrap()).unwrap();
        fs::write(bridge.service_binary_path(), b"compiled fixture").unwrap();
        let expected_hash = format!("{:x}", Sha256::digest(SERVICE_SOURCE.as_bytes()));
        fs::write(
            bridge.service_source_path().with_extension("sha256"),
            expected_hash,
        )
        .unwrap();

        let result = bridge.ensure_installed().await;
        assert!(
            !matches!(result, Ok(BridgeInstallResult { changed: false, .. })),
            "missing embedded source must trigger repair instead of an unchanged result"
        );
    }

    #[tokio::test]
    async fn request_ignores_unrelated_responses_and_validates_identity() {
        let root = tempdir().unwrap();
        let bridge = bridge_fixture(root.path());
        let responder = bridge.clone();
        let task = tokio::spawn(async move {
            responder.ensure_protocol_dirs().unwrap();
            let requests = responder.root().join("requests");
            for _ in 0..100 {
                if let Some(path) = fs::read_dir(&requests)
                    .ok()
                    .and_then(|entries| entries.filter_map(|entry| entry.ok()).next())
                    .map(|entry| entry.path())
                {
                    let request = parse_fields(&fs::read_to_string(&path).unwrap()).unwrap();
                    let request_id = request.get("request_id").unwrap().clone();
                    let unrelated = responder.root().join("responses").join("other.res");
                    atomic_write(&unrelated, b"not=this-request\n").unwrap();
                    let response = BTreeMap::from([
                        ("protocol".into(), BRIDGE_PROTOCOL_VERSION.into()),
                        ("request_id".into(), request_id.clone()),
                        ("instance_id".into(), responder.instance_id().into()),
                        ("ok".into(), "true".into()),
                    ]);
                    let destination = responder
                        .root()
                        .join("responses")
                        .join(format!("{}.res", request_id));
                    atomic_write(&destination, serialize_fields(&response).as_bytes()).unwrap();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("request was not published");
        });

        let response = bridge
            .request(
                "list_server_symbols",
                &BTreeMap::new(),
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        task.await.unwrap();
        assert!(response.bool("ok"));
        assert!(bridge.root().join("responses").join("other.res").exists());
    }

    #[tokio::test]
    async fn symbol_catalog_file_is_identity_bound_decoded_and_removed() {
        let root = tempdir().unwrap();
        let bridge = bridge_fixture(root.path());
        let responder = bridge.clone();
        let task = tokio::spawn(async move {
            responder.ensure_protocol_dirs().unwrap();
            for _ in 0..100 {
                if let Some(path) = fs::read_dir(responder.root().join("requests"))
                    .ok()
                    .and_then(|entries| entries.filter_map(|entry| entry.ok()).next())
                    .map(|entry| entry.path())
                {
                    let request = parse_fields(&fs::read_to_string(path).unwrap()).unwrap();
                    let request_id = request.get("request_id").unwrap();
                    let file_name = format!("{}.symbols", request_id);
                    atomic_write(
                        &responder.root().join("responses").join(&file_name),
                        b"EURUSDm\nXAUUSD%2Ecent\n",
                    )
                    .unwrap();
                    let response = BTreeMap::from([
                        ("protocol".into(), BRIDGE_PROTOCOL_VERSION.into()),
                        ("request_id".into(), request_id.clone()),
                        ("instance_id".into(), responder.instance_id().into()),
                        ("ok".into(), "true".into()),
                        ("symbols_file".into(), file_name),
                    ]);
                    atomic_write(
                        &responder
                            .root()
                            .join("responses")
                            .join(format!("{}.res", request_id)),
                        serialize_fields(&response).as_bytes(),
                    )
                    .unwrap();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("request was not published");
        });

        let symbols = bridge
            .list_server_symbols(Duration::from_secs(2))
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(symbols, vec!["EURUSDm", "XAUUSD.cent"]);
        assert!(fs::read_dir(bridge.root().join("responses"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".symbols")));
    }
}
