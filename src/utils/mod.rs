/// Read a file that may be UTF-16LE (with BOM) or UTF-8, returning a UTF-8 String.
/// MT5 .set and .ini files are typically UTF-16LE with BOM (0xFF 0xFE).
pub fn read_file_as_utf8(path: &std::path::Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)?;

    // Check for UTF-16LE BOM (0xFF 0xFE)
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        // UTF-16LE with BOM - skip the 2-byte BOM and decode
        let utf16_data: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16_data)
            .map_err(|e| anyhow::anyhow!("Failed to decode UTF-16LE: {}", e))
    } else {
        // Try UTF-8
        String::from_utf8(bytes).map_err(|e| anyhow::anyhow!("Failed to decode as UTF-8: {}", e))
    }
}

/// Write MT5 text using UTF-16LE with a BOM. MT5 uses this encoding for .set
/// and many .ini files on native Windows as well as under Wine.
pub fn write_file_utf16le(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    make_writable(path);
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(content.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Toggle the portable read-only flag without relying on Unix chmod.
pub fn set_readonly(path: &std::path::Path, readonly: bool) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_readonly(readonly);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

pub fn make_writable(path: &std::path::Path) {
    let _ = set_readonly(path, false);
}

#[cfg(target_os = "windows")]
fn tasklist_rows(output: &[u8]) -> Vec<(String, u32)> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.trim().trim_matches('"').split("\",\"").collect();
            let image = fields.first()?.trim_matches('"').to_string();
            let pid = fields.get(1)?.trim_matches('"').parse::<u32>().ok()?;
            Some((image, pid))
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_process_path(executable: &std::path::Path) -> String {
    let canonical = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let value = canonical.to_string_lossy();

    if let Some(unc_path) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc_path}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
    }
}

#[cfg(target_os = "windows")]
fn process_ids_for_executable(executable: &std::path::Path) -> Vec<u32> {
    if !executable.is_file() {
        return Vec::new();
    }

    let executable = windows_process_path(executable).replace('\'', "''");
    let image = std::path::Path::new(executable.as_str())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; Get-CimInstance Win32_Process -Filter \"Name='{}'\" | Where-Object {{ $_.ExecutablePath -and $_.ExecutablePath -ieq '{}' }} | ForEach-Object {{ $_.ProcessId }}",
        image, executable
    );

    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn configured_mt5_pids(config: &crate::models::Config) -> Vec<u32> {
    #[cfg(target_os = "windows")]
    {
        let mut pids = Vec::new();
        if let Some(terminal) = config.terminal_executable() {
            pids.extend(process_ids_for_executable(&terminal));
        }
        if let Some(tester) = config.metatester_executable() {
            pids.extend(process_ids_for_executable(&tester));
        }
        pids.sort_unstable();
        pids.dedup();
        return pids;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = config;
        Vec::new()
    }
}

pub fn is_configured_mt5_running(config: &crate::models::Config) -> bool {
    #[cfg(target_os = "windows")]
    {
        return !configured_mt5_pids(config).is_empty();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = config;
        is_mt5_running()
    }
}

pub fn kill_configured_mt5_processes(config: &crate::models::Config, force: bool) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        return configured_mt5_pids(config)
            .into_iter()
            .filter_map(|pid| {
                kill_pid(pid, force)
                    .err()
                    .map(|error| format!("{}: {}", pid, error))
            })
            .collect();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = config;
        kill_mt5_processes(force)
    }
}

pub fn is_process_running(image_name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let filter = format!("IMAGENAME eq {}", image_name);
        return std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
            .map(|output| {
                output.status.success()
                    && tasklist_rows(&output.stdout)
                        .iter()
                        .any(|(image, _)| image.eq_ignore_ascii_case(image_name))
            })
            .unwrap_or(false);
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("pgrep")
            .args(["-fi", image_name])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

pub fn is_mt5_running() -> bool {
    is_process_running("terminal64.exe") || is_process_running("metatester64.exe")
}

/// Stop the native/Wine MT5 process tree. Returns command failures for callers
/// that want to surface detailed diagnostics; "process not found" is harmless.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn kill_mt5_processes(force: bool) -> Vec<String> {
    let mut failures = Vec::new();

    #[cfg(target_os = "windows")]
    {
        for image in ["terminal64.exe", "metatester64.exe"] {
            let mut command = std::process::Command::new("taskkill");
            command.args(["/T", "/IM", image]);
            if force {
                command.arg("/F");
            }
            match command.output() {
                Ok(output) if !output.status.success() && is_process_running(image) => {
                    failures.push(format!(
                        "{}: {}",
                        image,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Err(error) => failures.push(format!("{}: {}", image, error)),
                _ => {}
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let signal = if force { "-KILL" } else { "-TERM" };
        for pattern in [
            "terminal64\\.exe",
            "metatester64\\.exe",
            "MetaTrader 5\\.app",
        ] {
            if let Err(error) = std::process::Command::new("pkill")
                .args([signal, "-f", pattern])
                .output()
            {
                failures.push(format!("{}: {}", pattern, error));
            }
        }
        if force {
            let _ = std::process::Command::new("pkill")
                .args(["-KILL", "-f", "wineserver"])
                .output();
        }
    }

    failures
}

pub fn is_pid_running(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let filter = format!("PID eq {}", pid);
        return std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
            .map(|output| {
                output.status.success()
                    && tasklist_rows(&output.stdout)
                        .iter()
                        .any(|(_, listed_pid)| *listed_pid == pid)
            })
            .unwrap_or(false);
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn kill_pid(pid: u32, force: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("taskkill");
        command.args(["/T", "/PID", &pid.to_string()]);
        if force {
            command.arg("/F");
        }
        let status = command.status()?;
        if !status.success() {
            anyhow::bail!("taskkill failed for PID {}", pid);
        }
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let signal = if force { "-9" } else { "-15" };
        let status = std::process::Command::new("kill")
            .args([signal, &pid.to_string()])
            .status()?;
        if !status.success() {
            anyhow::bail!("kill failed for PID {}", pid);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16le_round_trip_preserves_mt5_content() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("strategy.set");
        let content = "Lots=0.10||0.01||0.01||0.20||Y\r\nComment=Türkçe";

        write_file_utf16le(&path, content).expect("write UTF-16LE");

        let bytes = std::fs::read(&path).expect("read raw bytes");
        assert!(bytes.starts_with(&[0xFF, 0xFE]));
        assert_eq!(read_file_as_utf8(&path).expect("decode UTF-16LE"), content);
    }

    #[test]
    fn readonly_flag_can_be_applied_and_cleared() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("protected.set");
        write_file_utf16le(&path, "Risk=1").expect("create set file");

        set_readonly(&path, true).expect("set read-only");
        assert!(std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .readonly());

        make_writable(&path);
        assert!(!std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .readonly());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn tasklist_csv_parser_matches_pid_exactly() {
        let rows = tasklist_rows(b"\"terminal64.exe\",\"1234\",\"Console\",\"1\",\"33,432 K\"\r\n");
        assert_eq!(rows, vec![("terminal64.exe".to_string(), 1234)]);
        assert!(!rows.iter().any(|(_, pid)| *pid == 123));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn executable_pid_lookup_handles_windows_verbatim_paths() {
        let current_exe = std::env::current_exe().expect("current executable path");
        let pids = process_ids_for_executable(&current_exe);

        assert!(
            pids.contains(&std::process::id()),
            "CIM lookup did not find the current process: {pids:?}"
        );
    }
}
