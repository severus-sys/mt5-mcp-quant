use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationPass {
    pub pass: u32,
    pub profit: f64,
    pub total_trades: u32,
    pub profit_factor: f64,
    pub expected_payoff: f64,
    pub drawdown_pct: f64,
    pub params: HashMap<String, String>,
}

pub struct OptimizationParser;

impl OptimizationParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse_job(&self, job_id: &str) -> Result<Vec<OptimizationPass>> {
        let jobs_dir = std::env::temp_dir().join(".mt5_mcp_quant_jobs");
        let meta_path = jobs_dir.join(format!("{}.json", job_id));

        if !meta_path.exists() {
            return Err(anyhow!(
                "Job not found: {}. Check .mt5_mcp_quant_jobs/",
                job_id
            ));
        }

        let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(&meta_path)?)?;

        // Check if report_path is stored in metadata
        if let Some(report_base) = meta.get("report_path").and_then(|v| v.as_str()) {
            let base_path = Path::new(report_base);
            for ext in &[".htm", ".htm.xml", ".html"] {
                let candidate = base_path.with_extension(ext.trim_start_matches('.'));
                if candidate.exists() {
                    return self.parse_file(&candidate);
                }
            }
            return Err(anyhow!(
                "Optimization report not found at {}.*. Is MT5 optimization still running?",
                base_path.display()
            ));
        }

        // Fallback: derive from wine_prefix (legacy)
        let wine_prefix = meta
            .get("wine_prefix")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("wine_prefix not in job metadata"))?;

        let base_path = Path::new(wine_prefix).join("drive_c/mt5_mcp_quant_opt_report");

        // Try different extensions
        for ext in &[".htm", ".htm.xml", ".html"] {
            let candidate = base_path.with_extension(ext.trim_start_matches('.'));
            if candidate.exists() {
                return self.parse_file(&candidate);
            }
        }

        Err(anyhow!(
            "Optimization report not found. Expected: {}.htm or {}.htm.xml\nIs MT5 optimization still running?",
            base_path.display(),
            base_path.display()
        ))
    }

    pub fn parse_file(&self, path: &Path) -> Result<Vec<OptimizationPass>> {
        let format = self.detect_format(path);
        let text = self.read_text(path)?;

        match format {
            "xml" => self.parse_xml(&text),
            _ => self.parse_html(&text),
        }
    }

    fn detect_format(&self, path: &Path) -> &str {
        let path_str = path.to_string_lossy();
        if path_str.ends_with(".xml") || path_str.ends_with(".htm.xml") {
            return "xml";
        }

        if let Ok(header) = fs::read(path) {
            let header = &header[..header.len().min(512)];
            if header.windows(5).any(|w| w == b"<?xml")
                || header.windows(8).any(|w| w == b"Workbook")
            {
                return "xml";
            }
        }

        "html"
    }

    fn read_text(&self, path: &Path) -> Result<String> {
        let raw = fs::read(path)?;

        // Try UTF-16 first (common for MT5 reports)
        if raw.len() >= 2 {
            if raw[0] == 0xFF && raw[1] == 0xFE {
                // UTF-16 LE with BOM
                let u16_vec: Vec<u16> = raw[2..]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                return Ok(String::from_utf16_lossy(&u16_vec));
            } else if raw[0] == 0xFE && raw[1] == 0xFF {
                // UTF-16 BE with BOM
                let u16_vec: Vec<u16> = raw[2..]
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                return Ok(String::from_utf16_lossy(&u16_vec));
            }
        }

        // Try UTF-8, then fallback to lossy
        if let Ok(text) = String::from_utf8(raw.clone()) {
            return Ok(text);
        }

        // Try UTF-16 without BOM
        if raw.len() % 2 == 0 {
            let u16_vec: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let text = String::from_utf16_lossy(&u16_vec);
            if text.chars().any(|c| c.is_ascii_alphanumeric()) {
                return Ok(text);
            }
        }

        Ok(String::from_utf8_lossy(&raw).to_string())
    }

    fn parse_html(&self, text: &str) -> Result<Vec<OptimizationPass>> {
        let mut results = Vec::new();
        let mut headers: Vec<String> = Vec::new();

        // Find all table rows
        let row_regex = regex::Regex::new(r"<tr[^>]*>(.*?)</tr>")?;
        let cell_regex = regex::Regex::new(r"<t[dh][^>]*>(.*?)</t[dh]>")?;
        let tag_regex = regex::Regex::new(r"<[^>]+>")?;

        for row_caps in row_regex.captures_iter(text) {
            let row = &row_caps[1];
            let cells: Vec<String> = cell_regex
                .captures_iter(row)
                .map(|c| {
                    let cell = &c[1];
                    tag_regex
                        .replace_all(cell, "")
                        .trim()
                        .to_string()
                        .replace(',', "")
                })
                .collect();

            if cells.is_empty() {
                continue;
            }

            // Header row detection
            if headers.is_empty() && cells[0].to_lowercase().contains("pass") {
                headers = cells;
                continue;
            }

            // Data row
            if !headers.is_empty() && cells[0].parse::<u32>().is_ok() {
                let row_map: HashMap<String, String> = headers
                    .iter()
                    .zip(cells.iter())
                    .map(|(h, c)| (h.to_lowercase().replace(' ', "_"), c.clone()))
                    .collect();

                if let Some(pass) = self.row_to_pass(&row_map) {
                    results.push(pass);
                }
            }
        }

        Ok(results)
    }

    fn parse_xml(&self, text: &str) -> Result<Vec<OptimizationPass>> {
        let mut results = Vec::new();
        let mut headers: Vec<String> = Vec::new();

        // Parse SpreadsheetML XML
        let doc = roxmltree::Document::parse(text)?;

        // Find all rows in Worksheet/Table
        for node in doc.descendants() {
            if node.has_tag_name(("urn:schemas-microsoft-com:office:spreadsheet", "Row"))
                || node.has_tag_name("Row")
            {
                let mut cells: Vec<String> = Vec::new();
                for cell in node.children().filter(|child| {
                    child.has_tag_name(("urn:schemas-microsoft-com:office:spreadsheet", "Cell"))
                        || child.has_tag_name("Cell")
                }) {
                    let index = cell
                        .attribute(("urn:schemas-microsoft-com:office:spreadsheet", "Index"))
                        .or_else(|| cell.attribute("Index"))
                        .and_then(|value| value.parse::<usize>().ok());
                    if let Some(index) = index {
                        while cells.len() + 1 < index {
                            cells.push(String::new());
                        }
                    }
                    let value = cell
                        .descendants()
                        .find(|data| {
                            data.has_tag_name((
                                "urn:schemas-microsoft-com:office:spreadsheet",
                                "Data",
                            )) || data.has_tag_name("Data")
                        })
                        .and_then(|data| data.text())
                        .unwrap_or_default()
                        .trim()
                        .replace(',', "");
                    cells.push(value);
                }

                if cells.is_empty() {
                    continue;
                }

                if cells
                    .first()
                    .map(|value| Self::normalize_header(value) == "pass")
                    .unwrap_or(false)
                {
                    headers = cells;
                    continue;
                }

                // MT5 pass numbering starts at zero.
                if cells[0].parse::<u32>().is_ok() {
                    let mut row_map = HashMap::new();

                    let fallback_headers = [
                        "pass",
                        "result",
                        "profit",
                        "total_trades",
                        "profit_factor",
                        "expected_payoff",
                        "drawdown_pct",
                        "recovery_factor",
                        "sharpe_ratio",
                        "custom",
                        "consecutive_wins",
                        "consecutive_losses",
                    ];
                    for (index, cell) in cells.iter().enumerate() {
                        let header = headers
                            .get(index)
                            .map(String::as_str)
                            .or_else(|| fallback_headers.get(index).copied());
                        if let Some(header) = header {
                            row_map.insert(Self::normalize_header(header), cell.clone());
                        }
                    }

                    if let Some(mut pass) = self.row_to_pass(&row_map) {
                        if !headers.is_empty() {
                            pass.params = headers
                                .iter()
                                .zip(cells.iter())
                                .filter(|(header, _)| {
                                    !Self::is_standard_column(&Self::normalize_header(header))
                                })
                                .map(|(header, value)| (header.trim().to_string(), value.clone()))
                                .collect();
                        }
                        results.push(pass);
                    }
                }
            }
        }

        Ok(results)
    }

    fn normalize_header(header: &str) -> String {
        let normalized = header
            .trim()
            .trim_start_matches('#')
            .to_lowercase()
            .replace([' ', '-', '/', '.'], "_")
            .replace('%', "pct");
        let normalized = normalized.trim_matches('_').to_string();
        match normalized.as_str() {
            "" | "pass_number" => "pass".to_string(),
            "trades" | "total_deals" => "total_trades".to_string(),
            "total_net_profit" | "net_profit" => "profit".to_string(),
            "expected_profit" => "expected_payoff".to_string(),
            "drawdown" | "drawdown_pct" | "drawdown__pct" | "max_drawdown" | "equity_dd_pct"
            | "balance_dd_pct" => "drawdown_pct".to_string(),
            _ => normalized,
        }
    }

    fn is_standard_column(header: &str) -> bool {
        matches!(
            header,
            "pass"
                | "result"
                | "profit"
                | "total_trades"
                | "profit_factor"
                | "expected_payoff"
                | "drawdown_pct"
                | "max_drawdown"
                | "recovery_factor"
                | "sharpe_ratio"
                | "custom"
                | "consecutive_wins"
                | "consecutive_losses"
        )
    }

    fn row_to_pass(&self, row: &HashMap<String, String>) -> Option<OptimizationPass> {
        let pass = row
            .get("pass")
            .or_else(|| row.get("#"))
            .and_then(|v| v.parse().ok())?;

        let profit = row
            .get("profit")
            .or_else(|| row.get("total_net_profit"))
            .and_then(|v| v.replace(' ', "").parse().ok())?;

        let total_trades = row
            .get("total_trades")
            .or_else(|| row.get("trades"))
            .and_then(|v| v.parse().ok())?;

        let profit_factor = row.get("profit_factor").and_then(|v| v.parse().ok())?;

        let expected_payoff = row.get("expected_payoff").and_then(|v| v.parse().ok())?;

        let drawdown_pct = row
            .get("drawdown_pct")
            .or_else(|| row.get("max_drawdown"))
            .and_then(|v| v.trim_end_matches('%').trim().parse().ok())?;

        // Extract parameter values from row
        let params: HashMap<String, String> = row
            .iter()
            .filter(|(k, _)| {
                ![
                    "pass",
                    "result",
                    "profit",
                    "total_trades",
                    "profit_factor",
                    "expected_payoff",
                    "drawdown_pct",
                    "max_drawdown",
                    "recovery_factor",
                    "sharpe_ratio",
                    "custom",
                    "consecutive_wins",
                    "consecutive_losses",
                ]
                .contains(&k.as_str())
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Some(OptimizationPass {
            pass,
            profit,
            total_trades,
            profit_factor,
            expected_payoff,
            drawdown_pct,
            params,
        })
    }

    pub fn find_best_pass<'a>(
        &self,
        passes: &'a [OptimizationPass],
        criteria: &str,
    ) -> Option<&'a OptimizationPass> {
        match criteria {
            "profit" => passes
                .iter()
                .max_by(|a, b| a.profit.partial_cmp(&b.profit).unwrap()),
            "profit_factor" => passes
                .iter()
                .max_by(|a, b| a.profit_factor.partial_cmp(&b.profit_factor).unwrap()),
            "sharpe" => passes.iter().max_by(|a, b| {
                let a_sharpe = a
                    .params
                    .get("sharpe_ratio")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let b_sharpe = b
                    .params
                    .get("sharpe_ratio")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                a_sharpe.partial_cmp(&b_sharpe).unwrap()
            }),
            "drawdown" => passes
                .iter()
                .min_by(|a, b| a.drawdown_pct.partial_cmp(&b.drawdown_pct).unwrap()),
            _ => passes
                .iter()
                .max_by(|a, b| a.profit.partial_cmp(&b.profit).unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_xml_preserves_dynamic_ea_parameters() {
        let xml = r#"<?xml version="1.0"?>
<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet">
  <Worksheet><Table>
    <Row>
      <Cell><Data>Pass</Data></Cell><Cell><Data>Result</Data></Cell>
      <Cell><Data>Profit</Data></Cell><Cell><Data>Total Trades</Data></Cell>
      <Cell><Data>Profit Factor</Data></Cell><Cell><Data>Expected Payoff</Data></Cell>
      <Cell><Data>Equity DD %</Data></Cell><Cell><Data>TP_Pips</Data></Cell>
      <Cell><Data>UseFilter</Data></Cell>
    </Row>
    <Row>
      <Cell><Data>0</Data></Cell><Cell><Data>10</Data></Cell>
      <Cell><Data>125.5</Data></Cell><Cell><Data>8</Data></Cell>
      <Cell><Data>1.75</Data></Cell><Cell><Data>15.68</Data></Cell>
      <Cell><Data>4.2</Data></Cell><Cell><Data>400</Data></Cell>
      <Cell><Data>true</Data></Cell>
    </Row>
  </Table></Worksheet>
</Workbook>"#;

        let passes = OptimizationParser::new().parse_xml(xml).expect("parse XML");
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].pass, 0);
        assert_eq!(
            passes[0].params.get("TP_Pips").map(String::as_str),
            Some("400")
        );
        assert_eq!(
            passes[0].params.get("UseFilter").map(String::as_str),
            Some("true")
        );
    }
}
