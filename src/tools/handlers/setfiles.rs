use crate::models::Config;
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Read a file that may be UTF-16LE (with BOM) or UTF-8, returning a UTF-8 String.
fn read_file_as_utf8(path: &str) -> Result<String> {
    crate::utils::read_file_as_utf8(Path::new(path))
}

fn json_scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn render_parameter_value(value: &Value, existing_value: Option<&str>) -> Result<String> {
    let existing_parts: Vec<&str> = existing_value
        .map(|value| value.split("||").collect())
        .unwrap_or_default();
    let existing_is_sweep = existing_parts.len() >= 5
        && existing_parts
            .last()
            .map(|flag| flag.trim().eq_ignore_ascii_case("Y"))
            .unwrap_or(false);

    if let Some(object) = value.as_object() {
        let base = object
            .get("value")
            .and_then(json_scalar_to_string)
            .or_else(|| existing_parts.first().map(|value| value.trim().to_string()))
            .unwrap_or_else(|| "0".to_string());
        let optimize = object
            .get("optimize")
            .and_then(Value::as_bool)
            .unwrap_or(existing_is_sweep);

        if !optimize {
            return Ok(base);
        }

        let component = |key: &str, index: usize, fallback: &str| {
            object
                .get(key)
                .and_then(json_scalar_to_string)
                .or_else(|| {
                    existing_parts
                        .get(index)
                        .map(|value| value.trim().to_string())
                })
                .unwrap_or_else(|| fallback.to_string())
        };
        let from = component("from", 1, "0");
        let step = component("step", 2, "1");
        let to = component("to", 3, "0");
        return Ok(format!("{}||{}||{}||{}||Y", base, from, step, to));
    }

    let base = json_scalar_to_string(value).ok_or_else(|| {
        anyhow::anyhow!("set parameter values must be scalar or parameter objects")
    })?;
    let suffix = existing_value
        .and_then(|value| value.find("||").map(|index| &value[index..]))
        .unwrap_or("");
    Ok(format!("{}{}", base, suffix))
}

fn set_parameter_line(lines: &mut Vec<String>, key: &str, rendered_value: &str) -> bool {
    if let Some(line) = lines.iter_mut().find(|line| {
        line.split_once('=')
            .map(|(existing_key, _)| existing_key.trim() == key)
            .unwrap_or(false)
    }) {
        *line = format!("{}={}", key, rendered_value);
        true
    } else {
        lines.push(format!("{}={}", key, rendered_value));
        false
    }
}

fn strip_sweep(value: &str) -> &str {
    value.split("||").next().unwrap_or(value).trim()
}

fn write_set_file(path: &str, content: &str) -> Result<()> {
    let path = Path::new(path);
    crate::utils::write_file_utf16le(path, content)?;
    crate::utils::set_readonly(path, true)?;
    Ok(())
}

pub async fn handle_read_set_file(args: &Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;

    let content = read_file_as_utf8(path)?;
    let mut params = serde_json::Map::new();

    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            if value.contains("||Y") {
                let parts: Vec<&str> = value.split("||").collect();
                if parts.len() >= 5 {
                    params.insert(
                        key.to_string(),
                        json!({
                            "value": parts[0],
                            "from": parts[1],
                            "step": parts[2],
                            "to": parts[3],
                            "optimize": true,
                        }),
                    );
                }
            } else {
                params.insert(
                    key.to_string(),
                    json!({ "value": value, "optimize": false }),
                );
            }
        }
    }

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "path": path,
            "parameters": params,
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_write_set_file(args: &Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;

    let params = args
        .get("parameters")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("parameters object is required"))?;

    let mut lines = Vec::new();
    for (key, value) in params {
        lines.push(format!("{}={}", key, render_parameter_value(value, None)?));
    }

    write_set_file(path, &lines.join("\n"))?;

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "path": path,
            "parameters_written": lines.len(),
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_patch_set_file(args: &Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;

    let patches = args
        .get("patches")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("patches object is required"))?;

    let content = read_file_as_utf8(path)?;
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut patched_count = 0;

    for (key, value) in patches {
        if let Some(index) = lines.iter().position(|line| {
            line.split_once('=')
                .map(|(existing_key, _)| existing_key.trim() == key)
                .unwrap_or(false)
        }) {
            let existing_value = lines[index]
                .split_once('=')
                .map(|(_, value)| value.trim().to_string())
                .unwrap_or_default();
            let rendered = render_parameter_value(value, Some(&existing_value))?;
            lines[index] = format!("{}={}", key, rendered);
        } else {
            let rendered = render_parameter_value(value, None)?;
            lines.push(format!("{}={}", key, rendered));
        }
        patched_count += 1;
    }

    write_set_file(path, &lines.join("\n"))?;

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "path": path,
            "parameters_patched": patched_count,
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_clone_set_file(args: &Value) -> Result<Value> {
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("source is required"))?;

    let destination = args
        .get("destination")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("destination is required"))?;

    crate::utils::make_writable(Path::new(destination));
    fs::copy(source, destination)?;
    crate::utils::set_readonly(Path::new(destination), true)?;

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "source": source,
            "destination": destination,
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_diff_set_files(args: &Value) -> Result<Value> {
    let file_a = args
        .get("file_a")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("file_a is required"))?;

    let file_b = args
        .get("file_b")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("file_b is required"))?;

    let content_a = read_file_as_utf8(file_a)?;
    let content_b = read_file_as_utf8(file_b)?;

    let mut differences = Vec::new();

    let lines_a: Vec<&str> = content_a.lines().collect();
    let lines_b: Vec<&str> = content_b.lines().collect();
    for i in 0..lines_a.len().max(lines_b.len()) {
        let line_a = lines_a.get(i).copied();
        let line_b = lines_b.get(i).copied();
        if line_a != line_b {
            differences.push(json!({
                "line": i + 1,
                "file_a": line_a,
                "file_b": line_b,
            }));
        }
    }

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "file_a": file_a,
            "file_b": file_b,
            "differences": differences,
            "total_differences": differences.len(),
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_set_from_optimization(args: &Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;

    let params = args
        .get("params")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("params is required"))?;

    let template = args.get("template").and_then(|value| value.as_str());
    let mut lines: Vec<String> = if let Some(template_path) = template {
        read_file_as_utf8(template_path)?
            .lines()
            .map(|line| {
                line.split_once('=')
                    .map(|(key, value)| format!("{}={}", key.trim(), strip_sweep(value)))
                    .unwrap_or_else(|| line.to_string())
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut opt_params_applied = 0usize;
    for (key, value) in params {
        if let Some(rendered) = json_scalar_to_string(value) {
            set_parameter_line(&mut lines, key, &rendered);
            opt_params_applied += 1;
        }
    }

    let mut swept_params = 0usize;
    let mut total_combinations = 1u64;
    if let Some(sweep) = args.get("sweep").and_then(|value| value.as_object()) {
        for (key, range) in sweep {
            let Some(range) = range.as_object() else {
                continue;
            };
            let optimize = range
                .get("optimize")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if !optimize {
                continue;
            }
            let from = range.get("from").and_then(Value::as_f64).unwrap_or(0.0);
            let to = range.get("to").and_then(Value::as_f64).unwrap_or(from);
            let step = range.get("step").and_then(Value::as_f64).unwrap_or(0.0);
            if step <= 0.0 || to < from {
                anyhow::bail!(
                    "Invalid sweep range for '{}': from={}, to={}, step={}",
                    key,
                    from,
                    to,
                    step
                );
            }
            let base = params
                .get(key)
                .and_then(json_scalar_to_string)
                .or_else(|| {
                    lines.iter().find_map(|line| {
                        line.split_once('=')
                            .filter(|(existing_key, _)| existing_key.trim() == key)
                            .map(|(_, value)| strip_sweep(value).to_string())
                    })
                })
                .unwrap_or_else(|| from.to_string());
            let rendered = format!("{}||{}||{}||{}||Y", base, from, step, to);
            set_parameter_line(&mut lines, key, &rendered);
            let combinations = (((to - from) / step).floor() as u64).saturating_add(1);
            total_combinations = total_combinations.saturating_mul(combinations);
            swept_params += 1;
        }
    }
    if swept_params == 0 {
        total_combinations = 0;
    }

    write_set_file(path, &lines.join("\n"))?;

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "path": path,
            "param_count": lines.iter().filter(|line| line.split_once('=').is_some()).count(),
            "from_template": template.is_some(),
            "opt_params_applied": opt_params_applied,
            "swept_params": swept_params,
            "total_combinations": total_combinations,
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_describe_sweep(args: &Value) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;

    let content = read_file_as_utf8(path)?;
    let mut sweep_params = serde_json::Map::new();

    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            if value.contains("||Y") {
                let parts: Vec<&str> = value.split("||").collect();
                if parts.len() >= 5 && parts[4].trim().to_uppercase() == "Y" {
                    sweep_params.insert(
                        key.to_string(),
                        json!({
                            "from": parts[1].trim(),
                            "to": parts[3].trim(),
                            "step": parts[2].trim(),
                        }),
                    );
                }
            }
        }
    }

    Ok(json!({
        "content": [{ "type": "text", "text": json!({
            "success": true,
            "path": path,
            "sweep_params": sweep_params
        }).to_string() }],
        "isError": false
    }))
}

pub async fn handle_list_set_files(config: &Config) -> Result<Value> {
    let mut set_files = Vec::new();

    if let Some(tester_dir) = &config.tester_profiles_dir {
        if let Ok(entries) = fs::read_dir(tester_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "set").unwrap_or(false) {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let content = read_file_as_utf8(&path.to_string_lossy()).unwrap_or_default();
                    let param_count = content
                        .lines()
                        .filter(|line| {
                            let trimmed = line.trim();
                            !trimmed.is_empty()
                                && !trimmed.starts_with(';')
                                && trimmed
                                    .split_once('=')
                                    .map(|(key, _)| !key.trim().is_empty())
                                    .unwrap_or(false)
                        })
                        .count();
                    let sweep_count = content.lines().filter(|l| l.contains("||Y")).count();

                    set_files.push(json!({
                        "name": name,
                        "path": path.to_string_lossy(),
                        "param_count": param_count,
                        "sweep_count": sweep_count
                    }));
                }
            }
        }
    }

    Ok(json!({
        "content": [{ "type": "text", "text": json!({ "set_files": set_files }).to_string() }],
        "isError": false
    }))
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn regression_patch_set_file_renders_optimization_range_objects() {
        let dir = tempdir().expect("set tempdir");
        let path = dir.path().join("strategy.set");
        let path_string = path.to_string_lossy().into_owned();

        handle_write_set_file(&json!({
            "path": path_string,
            "parameters": {
                "Risk": {
                    "value": 1.0,
                    "from": 0.5,
                    "step": 0.5,
                    "to": 2.0,
                    "optimize": true
                }
            }
        }))
        .await
        .expect("write set fixture");

        handle_patch_set_file(&json!({
            "path": path_string,
            "patches": {
                "Risk": {
                    "value": 2.5,
                    "from": 1.0,
                    "step": 0.5,
                    "to": 4.0,
                    "optimize": true
                }
            }
        }))
        .await
        .expect("patch set file");

        let content = read_file_as_utf8(&path_string).expect("read patched set");
        assert_eq!(content.trim(), "Risk=2.5||1.0||0.5||4.0||Y");
    }
}
