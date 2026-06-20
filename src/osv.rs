//! OSV 漏洞扫描：批量查询已装包的已知漏洞（api.osv.dev，免费、无需 key）。

use crate::error::{PvmError, Result};
use std::io::Read;

#[derive(serde::Serialize)]
pub struct VulnHit {
    pub package: String,
    pub version: String,
    pub vuln_id: String,
    pub summary: String,
    pub severity: String,
    pub fixed: String,
}

/// 批量扫描 (name, version) 列表，返回命中的漏洞。
pub fn scan(packages: &[(String, String)]) -> Result<Vec<VulnHit>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    let queries: Vec<serde_json::Value> = packages
        .iter()
        .map(|(n, v)| serde_json::json!({"package":{"name":n,"ecosystem":"PyPI"},"version":v}))
        .collect();
    let body = serde_json::json!({ "queries": queries });
    let resp = crate::net::agent()
        .post("https://api.osv.dev/v1/querybatch")
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| PvmError::Http(format!("OSV 查询失败: {e}")))?;
    let val = read_body(resp)?;
    let results = val
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    let mut cache: std::collections::HashMap<String, (String, String, String)> =
        std::collections::HashMap::new();
    for (i, result) in results.iter().enumerate() {
        let (name, version) = match packages.get(i) {
            Some(p) => p,
            None => continue,
        };
        if let Some(vulns) = result.get("vulns").and_then(|v| v.as_array()) {
            for v in vulns {
                let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if id.is_empty() {
                    continue;
                }
                let detail = if let Some(d) = cache.get(&id) {
                    d.clone()
                } else {
                    let d = get_vuln(&id).unwrap_or_default();
                    cache.insert(id.clone(), d.clone());
                    d
                };
                out.push(VulnHit {
                    package: name.clone(),
                    version: version.clone(),
                    vuln_id: id,
                    summary: detail.0,
                    severity: detail.1,
                    fixed: detail.2,
                });
            }
        }
    }
    Ok(out)
}

fn get_vuln(id: &str) -> Result<(String, String, String)> {
    let resp = crate::net::agent()
        .get(&format!("https://api.osv.dev/v1/vulns/{id}"))
        .call()
        .map_err(|e| PvmError::Http(e.to_string()))?;
    let v = read_body(resp)?;
    let summary: String = v
        .get("summary")
        .or_else(|| v.get("details"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .chars()
        .take(240)
        .collect();
    let severity = v
        .get("severity")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("score").and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    let fixed = extract_fixed(&v);
    Ok((summary, severity, fixed))
}

fn extract_fixed(v: &serde_json::Value) -> String {
    let mut fixes = Vec::new();
    if let Some(affected) = v.get("affected").and_then(|a| a.as_array()) {
        for aff in affected {
            if let Some(ranges) = aff.get("ranges").and_then(|r| r.as_array()) {
                for rng in ranges {
                    if let Some(events) = rng.get("events").and_then(|e| e.as_array()) {
                        for ev in events {
                            if let Some(f) = ev.get("fixed").and_then(|x| x.as_str()) {
                                fixes.push(f.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    fixes.sort();
    fixes.dedup();
    fixes.join(", ")
}

fn read_body(resp: ureq::http::Response<ureq::Body>) -> Result<serde_json::Value> {
    let mut resp = resp;
    let mut s = String::new();
    resp.body_mut()
        .as_reader()
        .read_to_string(&mut s)
        .map_err(|e| PvmError::Http(e.to_string()))?;
    serde_json::from_str(&s).map_err(|e| PvmError::Http(format!("解析 OSV 响应失败: {e}")))
}
