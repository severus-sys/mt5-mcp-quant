use anyhow::{anyhow, Result};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::models::Config;
use crate::optimization::OptimizationParser;

/// Read a file that may be UTF-16LE (with BOM) or UTF-8, returning a UTF-8 String.
/// MT5 .set and .ini files are typically UTF-16LE with BOM (0xFF 0xFE).
fn read_file_as_utf8(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;

    // Check for UTF-16LE BOM (0xFF 0xFE)
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        // UTF-16LE with BOM - skip the 2-byte BOM and decode
        let utf16_data: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16_data).map_err(|e| anyhow!("Failed to decode UTF-16LE: {}", e))
    } else {
        // Try UTF-8
        String::from_utf8(bytes).map_err(|e| anyhow!("Failed to decode as UTF-8: {}", e))
    }
}

pub struct OptimizationParams {
    pub expert: String,
    pub set_file: String,
    pub symbol: String,
    pub from_date: String,
    pub to_date: String,
    pub deposit: u32,
    pub leverage: u32,
    pub currency: String,
    pub max_passes: Option<u32>,
    pub kill_existing: bool,
}

impl Default for OptimizationParams {
    fn default() -> Self {
        Self {
            expert: String::new(),
            set_file: String::new(),
            symbol: "XAUUSD".to_string(),
            from_date: String::new(),
            to_date: String::new(),
            deposit: 10000,
            leverage: 500,
            currency: "USD".to_string(),
            max_passes: None,
            kill_existing: false,
        }
    }
}

pub struct OptimizationResult {
    pub success: bool,
    pub job_id: String,
    pub pid: u32,
    pub log_file: PathBuf,
    pub combinations: u64,
    pub message: String,
}

pub struct OptimizationRunner {
    config: Config,
}

impl OptimizationRunner {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    fn validate_ini_value(label: &str, value: &str) -> Result<()> {
        if value.contains(['\r', '\n']) {
            return Err(anyhow!("{} contains an invalid newline", label));
        }
        Ok(())
    }

    pub async fn run(&self, params: OptimizationParams) -> Result<OptimizationResult> {
        // Validate required fields
        if params.expert.is_empty() {
            return Err(anyhow!("expert is required"));
        }
        if params.set_file.is_empty() {
            return Err(anyhow!("set_file is required"));
        }
        if params.from_date.is_empty() {
            return Err(anyhow!("from_date is required"));
        }
        if params.to_date.is_empty() {
            return Err(anyhow!("to_date is required"));
        }
        for (label, value) in [
            ("expert", params.expert.as_str()),
            ("set_file", params.set_file.as_str()),
            ("symbol", params.symbol.as_str()),
            ("from_date", params.from_date.as_str()),
            ("to_date", params.to_date.as_str()),
            ("currency", params.currency.as_str()),
        ] {
            Self::validate_ini_value(label, value)?;
        }

        let set_path = Path::new(&params.set_file);
        if !set_path.exists() {
            return Err(anyhow!("Set file not found: {}", params.set_file));
        }

        // MT5 is single-instance per installation. Never stop the configured
        // terminal unless the caller explicitly opted in.
        if crate::utils::is_configured_mt5_running(&self.config) {
            if !params.kill_existing {
                return Err(anyhow!(
                    "The configured MT5 instance is already running. Close it or set kill_existing=true."
                ));
            }
            let failures = crate::utils::kill_configured_mt5_processes(&self.config, true);
            if !failures.is_empty() {
                return Err(anyhow!(
                    "Could not stop configured MT5: {}",
                    failures.join("; ")
                ));
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        // Generate job ID and log file
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let job_id = format!("opt_{}", timestamp);
        let log_dir = self
            .config
            .opt_log_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("mt5-mcp-quant").join("logs"));
        fs::create_dir_all(&log_dir)?;
        let log_file = log_dir.join(format!("mt5opt_{}.log", timestamp));

        // Calculate agent count: 75% of available CPUs, or configured value
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let default_agents = ((cpu_count as f64 * 0.75).ceil() as u32).max(1);
        let max_agents = match self.config.opt_max_agents {
            Some(configured) if configured > 0 => configured,
            _ => default_agents,
        };

        // Count combinations
        let combinations = self
            .count_combinations(&params.set_file)
            .map_err(|e| anyhow!("count_combinations failed: {}", e))?;

        // Get paths
        let install_dir = self
            .config
            .terminal_install_dir()
            .ok_or_else(|| anyhow!("terminal_dir not configured"))?;
        let data_dir = self
            .config
            .mt5_dir()
            .ok_or_else(|| anyhow!("data_dir not configured"))?;

        // Resolve the .set filename MT5 will actually load: basename of set_file,
        // else "{expert}.set". Must match the name told to MT5 below (ExpertParameters).
        let set_param =
            if !params.set_file.is_empty() && params.set_file != format!("{}.set", params.expert) {
                std::path::Path::new(&params.set_file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&format!("{}.set", params.expert))
                    .to_string()
            } else {
                format!("{}.set", params.expert)
            };

        // Write .set file as UTF-16LE with BOM directly to MT5 tester directory
        let tester_dir = self
            .config
            .tester_profiles_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("MQL5").join("Profiles").join("Tester"));
        fs::create_dir_all(&tester_dir)
            .map_err(|e| anyhow!("create_dir_all({}) failed: {}", tester_dir.display(), e))?;
        let dst_set_file = tester_dir.join(&set_param);
        self.write_utf16le_set(&params.set_file, &dst_set_file)
            .map_err(|e| {
                anyhow!(
                    "write_utf16le_set({}) failed: {}",
                    dst_set_file.display(),
                    e
                )
            })?;

        // Reset OptMode in terminal.ini
        // Patch terminal.ini [Tester] section with optimization params (primary mechanism)
        let terminal_ini = if data_dir.join("config").exists() {
            data_dir.join("config").join("terminal.ini")
        } else {
            data_dir.join("terminal.ini")
        };
        let mt5_ini_text = if terminal_ini.exists() {
            read_file_as_utf8(&terminal_ini).unwrap_or_default()
        } else {
            String::new()
        };
        let expert_path = if let Some(experts_dir) = &self.config.experts_dir {
            let nested_dir = Path::new(experts_dir).join(&params.expert);
            if nested_dir.join(format!("{}.mq5", params.expert)).exists()
                || nested_dir.join(format!("{}.ex5", params.expert)).exists()
            {
                format!("{}\\{}.ex5", params.expert, params.expert)
            } else {
                format!("{}.ex5", params.expert)
            }
        } else {
            format!("{}.ex5", params.expert)
        };
        let reports_dir = data_dir.join("reports");
        fs::create_dir_all(&reports_dir)?;
        let report_base = reports_dir.join(format!("mt5_mcp_quant_{}", job_id));
        let report_ini = if cfg!(target_os = "windows") {
            format!("reports\\mt5_mcp_quant_{}.htm", job_id)
        } else {
            "..\\..\\mt5_mcp_quant_opt_report.htm".to_string()
        };
        let mut tester_section = format!(
            "[Tester]\n\
             Expert={}\n\
             ExpertParameters={}\n\
             Symbol={}\n\
             Period=M1\n\
             LocalAgents={}\n\
             Model=0\n\
             FromDate={}\n\
             ToDate={}\n\
             ForwardMode=0\n\
             Deposit={}\n\
             Currency={}\n\
             ProfitInPips=0\n\
             Leverage=1:{}\n\
             ExecutionMode=10\n\
              Optimization=2\n\
               OptimizationCriterion=0\n\
               Visual=0\n\
               Report={}\n\
               ReplaceReport=1\n\
                ShutdownTerminal=1",
            expert_path,
            set_param,
            params.symbol,
            max_agents,
            params.from_date,
            params.to_date,
            params.deposit,
            params.currency,
            params.leverage,
            report_ini,
        );
        if let Some(mp) = params.max_passes {
            tester_section.push_str(&format!("\nMaxPass={}", mp));
        }
        let updated_ini = Self::patch_ini_section(&mt5_ini_text, "Tester", &tester_section);
        // Strip any stale [Agents] sections from previous runs (no local agent processes)
        let cleaned = Self::strip_ini_section(&updated_ini, "Agents");
        let final_ini = cleaned.trim_end().to_string();
        let mut utf16_out: Vec<u8> = vec![0xFF, 0xFE];
        utf16_out.extend(final_ini.encode_utf16().flat_map(|c| c.to_le_bytes()));
        fs::write(&terminal_ini, utf16_out)?;

        // Write /config: INI to trigger tester/optimizer mode.
        let (opt_config_host, opt_config_arg) = self.optimization_config_paths(&install_dir)?;
        if let Some(parent) = opt_config_host.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut opt_ini = String::new();
        if let Some(login) = &self.config.backtest_login {
            if let Some(server) = &self.config.backtest_server {
                Self::validate_ini_value("login", login)?;
                Self::validate_ini_value("server", server)?;
                opt_ini.push_str("[Common]\n");
                opt_ini.push_str(&format!("Login={}\n", login));
                opt_ini.push_str(&format!("Server={}\n", server));
                // Native Windows can reuse the signed-in terminal session; do not
                // persist the account password in a temporary config file.
                if !cfg!(target_os = "windows") {
                    if let Some(password) = &self.config.backtest_password {
                        Self::validate_ini_value("password", password)?;
                        opt_ini.push_str(&format!("Password={}\n", password));
                    }
                }
                opt_ini.push_str("\n");
            }
        }
        opt_ini.push_str("[Tester]\n");
        opt_ini.push_str(&format!("Expert={}\n", expert_path));
        opt_ini.push_str(&format!("ExpertParameters={}\n", set_param));
        opt_ini.push_str(&format!("Symbol={}\n", params.symbol));
        opt_ini.push_str(&format!("Period={}\n", "M1"));
        opt_ini.push_str(&format!("LocalAgents={}\n", max_agents));
        opt_ini.push_str(&format!("Model={}\n", "0"));
        opt_ini.push_str("Optimization=2\n");
        opt_ini.push_str("OptimizationCriterion=0\n");
        opt_ini.push_str(&format!("FromDate={}\n", params.from_date));
        opt_ini.push_str(&format!("ToDate={}\n", params.to_date));
        opt_ini.push_str("ForwardMode=0\n");
        opt_ini.push_str(&format!("Deposit={}\n", params.deposit));
        opt_ini.push_str(&format!("Currency={}\n", params.currency));
        opt_ini.push_str("ProfitInPips=0\n");
        opt_ini.push_str(&format!("Leverage=1:{}\n", params.leverage));
        opt_ini.push_str("ExecutionMode=10\n");
        opt_ini.push_str("UseLocal=1\n");
        opt_ini.push_str("UseRemote=0\n");
        opt_ini.push_str("UseCloud=0\n");
        opt_ini.push_str("Visual=0\n");
        opt_ini.push_str(&format!("Report={}\n", report_ini));
        opt_ini.push_str("ReplaceReport=1\n");
        opt_ini.push_str("ShutdownTerminal=1\n");
        if let Some(mp) = params.max_passes {
            opt_ini.push_str(&format!("MaxPass={}\n", mp));
        }
        fs::write(&opt_config_host, opt_ini.as_bytes())?;

        let child = self.launch_optimizer(&install_dir, &opt_config_arg, max_agents)?;

        let pid = child.id();

        fs::write(
            &log_file,
            format!(
                "{} optimization launched: job={}, pid={}, expert={}, symbol={}, combinations={}\n",
                Utc::now().to_rfc3339(),
                job_id,
                pid,
                params.expert,
                params.symbol,
                combinations,
            ),
        )?;

        // Write job metadata
        self.write_job_metadata(
            &job_id,
            pid,
            &params,
            &log_file,
            combinations,
            &report_base,
            &opt_config_host,
        )?;

        Ok(OptimizationResult {
            success: true,
            job_id,
            pid,
            log_file,
            combinations,
            message: format!(
                "Optimization launched (pid: {}). Runs for 2-6 hours. Do NOT kill this process.",
                pid
            ),
        })
    }

    fn optimization_config_paths(&self, _install_dir: &Path) -> Result<(PathBuf, String)> {
        #[cfg(target_os = "windows")]
        {
            let host = std::env::temp_dir()
                .join("mt5-mcp-quant")
                .join(format!("mt5opt_{}.ini", uuid::Uuid::new_v4()));
            let argument = host.to_string_lossy().to_string();
            return Ok((host, argument));
        }

        #[cfg(not(target_os = "windows"))]
        {
            let prefix = self.get_wine_prefix_dir(_install_dir)?;
            Ok((
                prefix.join("drive_c").join("mt5opt_config.ini"),
                r"C:\mt5opt_config.ini".to_string(),
            ))
        }
    }

    fn launch_optimizer(
        &self,
        install_dir: &Path,
        config_argument: &str,
        _max_agents: u32,
    ) -> Result<std::process::Child> {
        #[cfg(target_os = "windows")]
        {
            let terminal = install_dir.join("terminal64.exe");
            if !terminal.is_file() {
                return Err(anyhow!(
                    "terminal64.exe not found at {}",
                    terminal.display()
                ));
            }
            return Command::new(&terminal)
                .arg(format!("/config:{}", config_argument))
                .current_dir(install_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| anyhow!("Failed to launch {}: {}", terminal.display(), error));
        }

        #[cfg(not(target_os = "windows"))]
        {
            let wine_exe = self
                .config
                .wine_executable
                .as_ref()
                .ok_or_else(|| anyhow!("wine_executable not configured"))?;
            let wine_prefix_dir = self.get_wine_prefix_dir(install_dir)?;
            let wine_bin = Path::new(wine_exe);
            let wine_root = wine_bin
                .parent()
                .and_then(|p| p.parent())
                .ok_or_else(|| anyhow!("Cannot derive Wine root from wine_exe"))?;
            let dyld = format!(
                "{}:{}:/usr/lib:/usr/local/lib",
                wine_root.join("lib").join("external").display(),
                wine_root.join("lib").display()
            );
            let terminal_host = wine_prefix_dir
                .join("drive_c")
                .join("Program Files")
                .join("MetaTrader 5")
                .join("terminal64.exe");
            let script = format!(
                "#!/bin/sh\n\
                 export DYLD_FALLBACK_LIBRARY_PATH='{dyld}'\n\
                 export WINEPREFIX='{prefix}'\n\
                 export WINEDEBUG='-all'\n\
                nohup taskset -c 0-$(( {max_agents} - 1 )) '{wine}' '{terminal}' '/config:{config}' >/dev/null 2>&1 &\n",
                dyld = dyld,
                prefix = wine_prefix_dir.display(),
                wine = wine_exe,
                terminal = terminal_host.display(),
                config = config_argument,
                max_agents = _max_agents,
            );
            let script_path = std::env::temp_dir().join("mt5opt_launch.sh");
            fs::write(&script_path, &script)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755))?;
            }
            Command::new("/bin/sh")
                .arg(&script_path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| {
                    anyhow!("spawn /bin/sh {} failed: {}", script_path.display(), error)
                })
        }
    }

    fn count_combinations(&self, set_file: &str) -> Result<u64> {
        let content = read_file_as_utf8(Path::new(set_file))?;
        let mut total: u64 = 1;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with(';') || !line.contains('=') {
                continue;
            }

            // Format: param=value||start||step||stop||Y
            let parts: Vec<&str> = line.split("||").collect();
            if parts.len() >= 5 && parts.last().unwrap().trim().to_uppercase() == "Y" {
                if let (Ok(start), Ok(step), Ok(stop)) = (
                    parts[1].trim().parse::<f64>(),
                    parts[2].trim().parse::<f64>(),
                    parts[3].trim().parse::<f64>(),
                ) {
                    if step > 0.0 {
                        let count = ((stop - start) / step).max(0.0) as u64 + 1;
                        total = total.saturating_mul(count);
                    }
                }
            }
        }

        Ok(total.max(1))
    }

    fn write_utf16le_set(&self, src: &str, dst: &Path) -> Result<()> {
        let content = read_file_as_utf8(Path::new(src))?;
        crate::utils::write_file_utf16le(dst, &content)?;
        crate::utils::set_readonly(dst, true)?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn get_wine_prefix_dir(&self, path: &Path) -> Result<PathBuf> {
        // Go up three levels: .../drive_c/Program Files/MetaTrader 5 -> .../net.metaquotes.wine.metatrader5
        // (same as backtest pipeline)
        let prefix_dir = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .ok_or_else(|| anyhow!("Cannot determine Wine prefix from terminal_dir"))?;
        Ok(prefix_dir.to_path_buf())
    }

    fn write_job_metadata(
        &self,
        job_id: &str,
        pid: u32,
        params: &OptimizationParams,
        log_file: &Path,
        combinations: u64,
        report_path: &Path,
        config_path: &Path,
    ) -> Result<()> {
        let jobs_dir = std::env::temp_dir().join(".mt5_mcp_quant_jobs");
        fs::create_dir_all(&jobs_dir)?;

        let meta_path = jobs_dir.join(format!("{}.json", job_id));
        let started_at = Utc::now().to_rfc3339();
        let metadata = serde_json::json!({
            "job_id": job_id,
            "pid": pid,
            "expert": params.expert,
            "symbol": params.symbol,
            "from_date": params.from_date,
            "to_date": params.to_date,
            "set_file": params.set_file,
            "combinations": combinations,
            "log_file": log_file.to_string_lossy(),
            "runtime": if cfg!(target_os = "windows") { "native-windows" } else { "wine" },
            "report_path": report_path.to_string_lossy(),
            "config_path": config_path.to_string_lossy(),
            "started_at": started_at,
        });

        fs::write(&meta_path, serde_json::to_string_pretty(&metadata)?)?;
        Ok(())
    }

    /// Replace a [section] in an INI string — removes old content and inserts new.
    fn patch_ini_section(text: &str, section: &str, new_content: &str) -> String {
        let section_header = format!("[{}]", section);
        let mut result = String::new();
        let mut in_section = false;
        let mut section_found = false;

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed == section_header {
                in_section = true;
                section_found = true;
                continue;
            }
            if in_section {
                if trimmed.starts_with('[') {
                    in_section = false;
                    result.push_str(new_content);
                    if !new_content.ends_with('\n') {
                        result.push('\n');
                    }
                    result.push_str(line);
                    result.push('\n');
                    continue;
                }
                continue;
            }
            result.push_str(line);
            result.push('\n');
        }

        if !section_found {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(new_content);
            result.push('\n');
        } else if in_section {
            result.push_str(new_content);
            result.push('\n');
        }

        result
    }

    /// Remove all lines belonging to a [section] from the INI text.
    fn strip_ini_section(text: &str, section: &str) -> String {
        let header = format!("[{}]", section);
        let mut result = String::new();
        let mut skipping = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed == header {
                skipping = true;
                continue;
            }
            if skipping && trimmed.starts_with('[') {
                skipping = false;
            }
            if !skipping {
                result.push_str(line);
                result.push('\n');
            }
        }
        result
    }

    pub fn get_job_status(&self, job_id: &str) -> Result<serde_json::Value> {
        let jobs_dir = std::env::temp_dir().join(".mt5_mcp_quant_jobs");
        let meta_path = jobs_dir.join(format!("{}.json", job_id));

        if !meta_path.exists() {
            return Ok(serde_json::json!({
                "status": "not_found",
                "message": format!("Job {} not found", job_id)
            }));
        }

        let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(&meta_path)?)?;
        if meta.get("status").and_then(|value| value.as_str()) == Some("completed") {
            return Ok(serde_json::json!({
                "status": "completed",
                "job_id": job_id,
                "pid": meta.get("pid"),
                "expert": meta.get("expert"),
                "symbol": meta.get("symbol"),
                "from_date": meta.get("from_date"),
                "to_date": meta.get("to_date"),
                "started_at": meta.get("started_at"),
                "completed_at": meta.get("completed_at"),
                "total_passes": meta.get("total_passes"),
                "top_10": meta.get("top_10"),
                "best_pf": meta.get("best_pf"),
            }));
        }

        let pid = meta.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let is_running = self.is_process_running(pid);

        let mut result = serde_json::json!({
            "status": if is_running { "running" } else { "stopped" },
            "job_id": job_id,
            "pid": pid,
            "expert": meta.get("expert"),
            "symbol": meta.get("symbol"),
            "from_date": meta.get("from_date"),
            "to_date": meta.get("to_date"),
            "started_at": meta.get("started_at"),
        });

        // If not running, try to parse the optimization report
        if !is_running {
            let parser = OptimizationParser::new();
            match parser.parse_job(job_id) {
                Ok(passes) if !passes.is_empty() => {
                    let mut sorted_by_pf = passes.clone();
                    sorted_by_pf
                        .sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap());
                    let top10: Vec<_> = sorted_by_pf.into_iter().take(10).collect();

                    let best_pf = parser.find_best_pass(&passes, "profit_factor");
                    let best_profit = parser.find_best_pass(&passes, "profit");

                    let m = result
                        .as_object_mut()
                        .ok_or_else(|| anyhow!("result is not object"))?;
                    m.insert(
                        "status".into(),
                        serde_json::Value::String("completed".into()),
                    );
                    m.insert("total_passes".into(), serde_json::json!(passes.len()));
                    m.insert(
                        "top_10".into(),
                        serde_json::to_value(&top10).unwrap_or_default(),
                    );
                    m.insert(
                        "best_pf".into(),
                        serde_json::to_value(best_pf).unwrap_or_default(),
                    );
                    m.insert(
                        "best_profit".into(),
                        serde_json::to_value(best_profit).unwrap_or_default(),
                    );
                }
                _ => {
                    let m = result
                        .as_object_mut()
                        .ok_or_else(|| anyhow!("result is not object"))?;
                    m.insert("status".into(), serde_json::Value::String("stopped".into()));
                    m.insert("message".into(), serde_json::Value::String(
                        "Optimization stopped but no report found — may have crashed or was killed early".into()
                    ));
                }
            }
            if let Some(config_path) = meta.get("config_path").and_then(|value| value.as_str()) {
                let _ = fs::remove_file(config_path);
            }
        }

        Ok(result)
    }

    fn is_process_running(&self, pid: u32) -> bool {
        crate::utils::is_pid_running(pid)
    }

    pub fn list_jobs(&self) -> Result<Vec<serde_json::Value>> {
        let jobs_dir = std::env::temp_dir().join(".mt5_mcp_quant_jobs");
        let mut jobs = Vec::new();

        if jobs_dir.exists() {
            for entry in fs::read_dir(jobs_dir)? {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&content) {
                                let job_id = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();

                                let status = self.get_job_status(&job_id).unwrap_or_else(|_| {
                                    serde_json::json!({
                                        "status": "unknown",
                                        "job_id": job_id,
                                    })
                                });
                                jobs.push(serde_json::json!({
                                    "job_id": job_id,
                                    "expert": status.get("expert").or_else(|| meta.get("expert")),
                                    "status": status.get("status").and_then(|value| value.as_str()).unwrap_or("unknown"),
                                    "started_at": status.get("started_at").or_else(|| meta.get("started_at")),
                                    "completed_at": status.get("completed_at").or_else(|| meta.get("completed_at")),
                                    "total_passes": status.get("total_passes").or_else(|| meta.get("total_passes")),
                                }));
                            }
                        }
                    }
                }
            }
        }

        Ok(jobs)
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    #[test]
    fn regression_list_jobs_preserves_completed_status() {
        let job_id = format!("regression_list_jobs_{}", uuid::Uuid::new_v4());
        let jobs_dir = std::env::temp_dir().join(".mt5_mcp_quant_jobs");
        fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        let meta_path = jobs_dir.join(format!("{}.json", job_id));
        fs::write(
            &meta_path,
            serde_json::to_string(&serde_json::json!({
                "job_id": job_id,
                "pid": u32::MAX,
                "expert": "RegressionEA",
                "status": "completed",
                "started_at": "2026-08-23T00:00:00Z",
                "completed_at": "2026-08-23T00:01:00Z",
                "total_passes": 2
            }))
            .expect("serialize metadata"),
        )
        .expect("write metadata");

        let runner = OptimizationRunner::new(Config::default());
        let jobs = runner.list_jobs().expect("list jobs");
        let _ = fs::remove_file(&meta_path);
        let job = jobs
            .iter()
            .find(|entry| entry["job_id"] == job_id)
            .expect("regression job in list");

        assert_eq!(job["status"], "completed");
        assert_eq!(job["total_passes"], 2);
    }
}
