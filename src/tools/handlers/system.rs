use crate::models::Config;
use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

// ── Update helpers ────────────────────────────────────────────────────────────

fn platform_tag() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-aarch64";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x86_64";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x86_64";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "windows-x64";
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    return "unsupported";
}

fn semver_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut p = s.trim_start_matches('v').splitn(3, '.');
        let ma = p.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let mi = p.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let pa = p.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        (ma, mi, pa)
    };
    parse(latest) > parse(current)
}

/// Fetch the latest release tag from GitHub API (5 s timeout via curl).
/// Returns the version string without the leading "v", or None on failure.
pub(super) async fn fetch_latest_version() -> Option<String> {
    let output = tokio::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "5",
            "-H",
            "Accept: application/vnd.github.v3+json",
            "-H",
            "User-Agent: mt5-mcp-quant-updater",
            "https://api.github.com/repos/severus-sys/mt5-mcp-quant/releases/latest",
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body: Value = serde_json::from_slice(&output.stdout).ok()?;
    body.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim_start_matches('v').to_string())
}

fn ok_response(data: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": data.to_string() }], "isError": false })
}

fn err_response(msg: impl std::fmt::Display) -> Value {
    json!({ "content": [{ "type": "text", "text": msg.to_string() }], "isError": true })
}

// ── Update tool handlers ──────────────────────────────────────────────────────

pub async fn handle_check_update(_config: &Config) -> Result<Value> {
    let current = env!("CARGO_PKG_VERSION");

    // Use cached background-check result if available; otherwise fetch now.
    let latest_opt = match super::LATEST_VERSION.get() {
        Some(v) => v.clone(),
        None => fetch_latest_version().await,
    };

    let Some(latest) = latest_opt else {
        return Ok(ok_response(json!({
            "current_version": current,
            "update_available": false,
            "error": "Could not reach GitHub API — check network connectivity",
        })));
    };

    let update_available = semver_newer(&latest, current);
    Ok(ok_response(json!({
        "current_version": current,
        "latest_version": latest,
        "update_available": update_available,
        "hint": if update_available {
            format!("Run the `update` tool to install v{latest}")
        } else {
            "You are on the latest version".to_string()
        },
    })))
}

pub async fn handle_update(_config: &Config, args: &Value) -> Result<Value> {
    let current = env!("CARGO_PKG_VERSION");

    if args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(ok_response(json!({
            "success": true,
            "dry_run": true,
            "current_version": current,
            "platform": platform_tag(),
            "message": "Update preflight passed; no files were downloaded or replaced."
        })));
    }

    let latest = match super::LATEST_VERSION.get().and_then(|v| v.as_deref()) {
        Some(v) => v.to_string(),
        None => match fetch_latest_version().await {
            Some(v) => v,
            None => {
                return Ok(err_response(
                    r#"{"success":false,"error":"Could not determine latest version — check network"}"#,
                ))
            }
        },
    };

    if !semver_newer(&latest, current) {
        return Ok(ok_response(json!({
            "up_to_date": true,
            "version": current,
        })));
    }

    let tag = platform_tag();
    if tag == "unsupported" {
        return Ok(err_response(
            r#"{"success":false,"error":"Auto-update not supported on this platform — build from source"}"#,
        ));
    }

    let file_name = format!("mcp-mt5-mcp-quant-{tag}.tar.gz");
    let Some((url, expected_sha256)) = fetch_release_asset(&latest, &file_name).await else {
        return Ok(err_response(
            r#"{"success":false,"error":"Release asset or trusted SHA-256 digest is unavailable"}"#,
        ));
    };

    // Download tarball to a temp file
    let tmp_tar = tempfile::NamedTempFile::new()?;
    let dl = tokio::process::Command::new("curl")
        .args([
            "-sfL",
            "--max-time",
            "120",
            "-o",
            tmp_tar.path().to_str().unwrap_or_default(),
            &url,
        ])
        .status()
        .await?;

    if !dl.success() {
        return Ok(err_response(format!(
            r#"{{"success":false,"error":"Download failed","url":"{}"}}"#,
            url
        )));
    }

    let archive = std::fs::read(tmp_tar.path())?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&archive));
    if actual_sha256 != expected_sha256 {
        return Ok(err_response(json!({
            "success": false,
            "error": "Downloaded release archive failed SHA-256 verification",
            "expected_sha256": expected_sha256,
            "actual_sha256": actual_sha256,
        })));
    }

    let listing = tokio::process::Command::new("tar")
        .args(["-tzf", tmp_tar.path().to_str().unwrap_or_default()])
        .output()
        .await?;
    if !listing.status.success() {
        return Ok(err_response(
            r#"{"success":false,"error":"Could not inspect update archive"}"#,
        ));
    }
    for entry in String::from_utf8_lossy(&listing.stdout).lines() {
        let path = Path::new(entry);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Ok(err_response(
                r#"{"success":false,"error":"Unsafe path in update archive"}"#,
            ));
        }
    }

    // Extract binary (tarball root dir is mcp-mt5-mcp-quant-{platform}/)
    let tmp_dir = tempfile::tempdir()?;
    let extract = tokio::process::Command::new("tar")
        .args([
            "-xzf",
            tmp_tar.path().to_str().unwrap_or_default(),
            "-C",
            tmp_dir.path().to_str().unwrap_or_default(),
            "--strip-components=1",
        ])
        .status()
        .await?;

    if !extract.success() {
        return Ok(err_response(
            r#"{"success":false,"error":"Failed to extract archive"}"#,
        ));
    }

    let new_bin = tmp_dir.path().join(if cfg!(target_os = "windows") {
        "mt5-mcp-quant.exe"
    } else {
        "mt5-mcp-quant"
    });
    if !new_bin.exists() {
        return Ok(err_response(
            r#"{"success":false,"error":"Binary not found in archive"}"#,
        ));
    }

    // Atomic replace on Unix. Windows locks the running executable, so stage
    // the replacement and let a detached helper swap it after this process exits.
    let current_exe = std::env::current_exe()?;
    let tmp_dest = current_exe.with_extension("update_tmp");
    std::fs::copy(&new_bin, &tmp_dest)?;

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let helper = std::env::temp_dir().join("mt5-mcp-quant-update.cmd");
        let script = format!(
            "@echo off\r\n\
             :wait\r\n\
             tasklist /FI \"PID eq {pid}\" /NH | find \"{pid}\" >nul\r\n\
             if not errorlevel 1 (timeout /t 1 /nobreak >nul & goto wait)\r\n\
             move /Y \"{staged}\" \"{current}\" >nul\r\n\
             del \"%~f0\"\r\n",
            pid = std::process::id(),
            staged = tmp_dest.display(),
            current = current_exe.display(),
        );
        std::fs::write(&helper, script)?;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        std::process::Command::new("cmd.exe")
            .args(["/C", helper.to_string_lossy().as_ref()])
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        return Ok(ok_response(json!({
            "success": true,
            "previous_version": current,
            "updated_to": latest,
            "binary": current_exe.to_string_lossy(),
            "hint": format!("v{latest} is staged. Restart the MCP connection to complete the executable swap."),
        })));
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp_dest, std::fs::Permissions::from_mode(0o755))?;
        }

        std::fs::rename(&tmp_dest, &current_exe)?;

        Ok(ok_response(json!({
            "success": true,
            "previous_version": current,
            "updated_to": latest,
            "binary": current_exe.to_string_lossy(),
            "hint": format!("Updated to v{latest}. Restart the MCP connection to load the new binary."),
        })))
    }
}

async fn fetch_release_asset(version: &str, file_name: &str) -> Option<(String, String)> {
    let endpoint = format!(
        "https://api.github.com/repos/severus-sys/mt5-mcp-quant/releases/tags/v{}",
        version
    );
    let output = tokio::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: mt5-mcp-quant-updater",
            &endpoint,
        ])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let release: Value = serde_json::from_slice(&output.stdout).ok()?;
    let asset = release
        .get("assets")?
        .as_array()?
        .iter()
        .find(|asset| asset.get("name").and_then(Value::as_str) == Some(file_name))?;
    let url = asset.get("browser_download_url")?.as_str()?.to_string();
    let digest = asset
        .get("digest")?
        .as_str()?
        .strip_prefix("sha256:")?
        .to_ascii_lowercase();
    if digest.len() != 64
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    Some((url, digest))
}

pub async fn handle_verify_setup(config: &Config) -> Result<Value> {
    let mut checks = serde_json::Map::new();
    let mut all_ok = true;

    let config_path = Config::writable_config_path();
    let config_exists = config_path.exists();
    checks.insert(
        "config_file".into(),
        json!({
            "ok": config_exists,
            "path": config_path.to_string_lossy()
        }),
    );

    let check = |v: &Option<String>, is_dir: bool| -> Value {
        match v {
            None => json!({ "ok": false, "detail": "not set" }),
            Some(p) => {
                let ok = if is_dir {
                    Path::new(p).is_dir()
                } else {
                    Path::new(p).exists()
                };
                json!({ "ok": ok, "detail": p })
            }
        }
    };

    let wine_ok = !Config::requires_wine()
        || config
            .wine_executable
            .as_ref()
            .map(|p| Path::new(p).exists())
            .unwrap_or(false);
    let term_ok = config
        .terminal_executable()
        .map(|p| p.is_file())
        .unwrap_or(false);
    let editor_ok = config
        .metaeditor_executable()
        .map(|p| p.is_file())
        .unwrap_or(false);
    let tester_ok = config
        .metatester_executable()
        .map(|p| p.is_file())
        .unwrap_or(false);
    let data_ok = config.mt5_dir().map(|p| p.is_dir()).unwrap_or(false);
    let experts_ok = config
        .experts_dir
        .as_ref()
        .map(|path| Path::new(path).is_dir())
        .unwrap_or(false);
    let profiles_ok = config
        .tester_profiles_dir
        .as_ref()
        .map(|path| Path::new(path).is_dir())
        .unwrap_or(false);

    if !config_exists
        || !wine_ok
        || !term_ok
        || !editor_ok
        || !tester_ok
        || !data_ok
        || !experts_ok
        || !profiles_ok
    {
        all_ok = false;
    }

    checks.insert(
        "runtime".into(),
        json!({
            "ok": wine_ok,
            "kind": if Config::requires_wine() { "wine" } else { "native-windows" },
            "wine_required": Config::requires_wine(),
            "wine_executable": config.wine_executable,
        }),
    );
    checks.insert("terminal_dir".into(), check(&config.terminal_dir, true));
    checks.insert("data_dir".into(), check(&config.data_dir, true));
    checks.insert(
        "terminal64_exe".into(),
        json!({
            "ok": term_ok,
            "detail": config.terminal_executable().map(|path| path.to_string_lossy().to_string()),
        }),
    );
    checks.insert(
        "metaeditor64_exe".into(),
        json!({
            "ok": editor_ok,
            "detail": config.metaeditor_executable().map(|path| path.to_string_lossy().to_string()),
        }),
    );
    checks.insert(
        "metatester64_exe".into(),
        json!({
            "ok": tester_ok,
            "detail": config.metatester_executable().map(|path| path.to_string_lossy().to_string()),
        }),
    );
    checks.insert("experts_dir".into(), check(&config.experts_dir, true));
    checks.insert("indicators_dir".into(), check(&config.indicators_dir, true));
    checks.insert("scripts_dir".into(), check(&config.scripts_dir, true));
    checks.insert(
        "tester_profiles_dir".into(),
        check(&config.tester_profiles_dir, true),
    );
    checks.insert("display_mode".into(), json!(config.display_mode));
    checks.insert(
        "reports_dir".into(),
        json!(config.reports_dir().to_string_lossy().to_string()),
    );
    checks.insert(
        "db_path".into(),
        json!(Config::db_path().to_string_lossy().to_string()),
    );

    let hint = if all_ok {
        "Environment fully configured and ready".into()
    } else if !config_path.exists() {
        format!(
            "Auto-discovery will run on next request. Config will be written to {}",
            config_path.display()
        )
    } else {
        format!("Fix missing paths in {}", config_path.display())
    };

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "all_ok": all_ok,
            "config_path": config_path.to_string_lossy(),
            "checks": checks,
            "hint": hint,
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_list_symbols(config: &Config) -> Result<Value> {
    // Get active account info
    let current_account = config.current_account();
    let active_server = current_account.as_ref().map(|a| a.server.clone());

    // Get all available servers for reference
    let all_servers = config.available_servers();

    // Get symbols for active server (or all if no active account)
    let symbols = config.discover_symbols_for_active_account();

    let hint = if symbols.is_empty() {
        if active_server.is_some() {
            "No history data found for the active account's server. Open MT5 and download tick data for the symbols you want to backtest."
        } else {
            "No history data found. Open MT5 and download tick data for the symbols you want to backtest."
        }
    } else {
        "These symbols have local tick history and can be used for backtesting."
    };

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "count": symbols.len(),
            "symbols": symbols,
            "active_account": current_account.map(|a| json!({
                "login": a.login,
                "server": a.server
            })),
            "active_server": active_server,
            "available_servers": all_servers,
            "hint": hint,
        }).to_string() }],
        "isError": false
    }))
}

/// Get active MT5 account info with available symbols for pre-flight checks
pub async fn handle_get_active_account(config: &Config) -> Result<Value> {
    let current_account = config.current_account();
    let active_server = current_account.as_ref().map(|a| a.server.clone());

    // Get all available servers
    let all_servers = config.available_servers();

    // Get symbols for active server (or all if no active account)
    let symbols = config.discover_symbols_for_active_account();

    // Determine readiness for backtesting
    let ready_for_backtest = current_account.is_some() && !symbols.is_empty();

    let hint = if current_account.is_none() {
        "No active MT5 account detected. Open MT5 and login to an account first."
    } else if symbols.is_empty() {
        "Active account found but no symbol history data. Download tick data in MT5 Strategy Tester."
    } else {
        "Ready for backtesting. Use these symbols with run_backtest."
    };

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "ready_for_backtest": ready_for_backtest,
            "account": current_account.map(|a| json!({
                "login": a.login,
                "server": a.server
            })),
            "server": active_server,
            "available_servers": all_servers,
            "symbols": symbols,
            "symbol_count": symbols.len(),
            "hint": hint,
        }).to_string() }],
        "isError": false
    }))
}

// OS Detection structs and healthcheck
#[derive(Debug)]
struct OsInfo {
    platform: String,
    arch: String,
    name: String,
    is_macos: bool,
    is_linux: bool,
    is_windows: bool,
}

#[derive(Debug)]
struct ConfigStatus {
    config_exists: bool,
    config_path: String,
    wine_found: bool,
    wine_path: Option<String>,
    mt5_dir_found: bool,
    mt5_dir: Option<String>,
    experts_dir_found: bool,
    indicators_dir_found: bool,
    scripts_dir_found: bool,
    tester_profiles_found: bool,
}

pub async fn handle_healthcheck(config: &Config, args: &Value) -> Result<Value> {
    let detailed = args
        .get("detailed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let os_info = detect_os();
    let config_status = validate_configuration(config).await;

    let mut healthy = true;
    let mut issues = Vec::new();

    if !config_status.config_exists {
        healthy = false;
        issues.push("Configuration file not found - run setup to configure");
    }
    if Config::requires_wine() && !config_status.wine_found {
        healthy = false;
        issues.push("Wine/CrossOver not found - required for MT5 execution");
    }
    if !config_status.mt5_dir_found {
        healthy = false;
        issues.push("MT5 directory not found - check installation");
    }

    let mut response = json!({
        "success": true,
        "healthy": healthy,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "os": {
            "platform": os_info.platform,
            "arch": os_info.arch,
            "name": os_info.name,
            "is_macos": os_info.is_macos,
            "is_linux": os_info.is_linux,
            "is_windows": os_info.is_windows,
        },
        "configuration": {
            "config_exists": config_status.config_exists,
            "config_path": config_status.config_path,
            "wine_found": config_status.wine_found,
            "wine_path": config_status.wine_path,
            "mt5_dir_found": config_status.mt5_dir_found,
            "mt5_dir": config_status.mt5_dir,
            "experts_dir_found": config_status.experts_dir_found,
            "indicators_dir_found": config_status.indicators_dir_found,
            "scripts_dir_found": config_status.scripts_dir_found,
            "tester_profiles_found": config_status.tester_profiles_found,
        },
        "issues": issues,
    });

    if detailed {
        response["detailed"] = json!({
            "rust_version": get_rust_version(),
            "exe_path": std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            "working_dir": std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            "env_vars": {
                "DISPLAY": std::env::var("DISPLAY").ok(),
                "WINEPREFIX": std::env::var("WINEPREFIX").ok(),
                "HOME": std::env::var("HOME").ok(),
                "USERPROFILE": std::env::var("USERPROFILE").ok(),
            },
        });
    }

    Ok(json!({
        "content": [{ "type": "text", "text": response.to_string() }],
        "isError": false
    }))
}

fn detect_os() -> OsInfo {
    let platform = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();

    let is_macos = platform == "macos";
    let is_linux = platform == "linux";
    let is_windows = platform == "windows";

    let name = if is_macos {
        get_macos_version().unwrap_or_else(|| "macOS".to_string())
    } else if is_linux {
        get_linux_distro().unwrap_or_else(|| "Linux".to_string())
    } else if is_windows {
        get_windows_version().unwrap_or_else(|| "Windows".to_string())
    } else {
        platform.clone()
    };

    OsInfo {
        platform,
        arch,
        name,
        is_macos,
        is_linux,
        is_windows,
    }
}

fn get_windows_version() -> Option<String> {
    std::process::Command::new("cmd.exe")
        .args(["/C", "ver"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn get_macos_version() -> Option<String> {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| format!("macOS {}", s.trim()))
}

fn get_linux_distro() -> Option<String> {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| l.replace("PRETTY_NAME=", "").trim_matches('"').to_string())
        })
}

fn get_rust_version() -> Option<String> {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

async fn validate_configuration(config: &Config) -> ConfigStatus {
    let config_path = Config::writable_config_path();
    let config_exists = config_path.exists();

    let wine_found = !Config::requires_wine()
        || config
            .wine_executable
            .as_ref()
            .map(|p| Path::new(p).exists())
            .unwrap_or(false);
    let wine_path = config.wine_executable.clone();

    let mt5_dir_found = config
        .terminal_executable()
        .map(|p| p.is_file())
        .unwrap_or(false);
    let mt5_dir = config.terminal_dir.clone();

    let experts_dir_found = config
        .experts_dir
        .as_ref()
        .map(|p| Path::new(p).is_dir())
        .unwrap_or(false);

    let indicators_dir_found = config
        .indicators_dir
        .as_ref()
        .map(|p| Path::new(p).is_dir())
        .unwrap_or(false);

    let scripts_dir_found = config
        .scripts_dir
        .as_ref()
        .map(|p| Path::new(p).is_dir())
        .unwrap_or(false);

    let tester_profiles_found = config
        .tester_profiles_dir
        .as_ref()
        .map(|p| Path::new(p).is_dir())
        .unwrap_or(false);

    ConfigStatus {
        config_exists,
        config_path: config_path.to_string_lossy().to_string(),
        wine_found,
        wine_path,
        mt5_dir_found,
        mt5_dir,
        experts_dir_found,
        indicators_dir_found,
        scripts_dir_found,
        tester_profiles_found,
    }
}
