//! Python 包管理。
//!
//! 性能关键：列包与本地详情**不启动 python**，直接用 Rust 扫描 site-packages 的
//! `*.dist-info` / `*.egg-info`（目录名即含 name-version，METADATA 即含元数据），
//! 比启动 pip/python 快一个数量级。仅安装/卸载/升级/过时检测才调 pip。
//! 详情分两步：本地（纯 Rust，秒出）+ PyPI（网络，异步补充可用版本/README）。

use crate::error::{PvmError, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Serialize, serde::Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct Outdated {
    pub name: String,
    pub version: String,
    pub latest_version: String,
}

#[derive(Serialize, Default)]
pub struct PkgDetail {
    pub name: String,
    pub version: String,
    pub summary: String,
    pub location: String,
    pub requires: Vec<String>,
    pub required_by: Vec<String>,
    pub author: String,
    pub license: String,
    pub home_page: String,
}

#[derive(Serialize, Default)]
pub struct PypiInfo {
    pub summary: String,
    pub author: String,
    pub license: String,
    pub home_page: String,
    pub project_urls: Vec<(String, String)>,
    pub available_versions: Vec<String>,
    pub readme: String,
}

// ---------- 纯 Rust：site-packages 扫描 ----------

/// 由 python.exe 路径推断 site-packages 目录（venv 的 Scripts/ 或 prefix 根）。
fn site_packages_dirs(py: &Path) -> Vec<PathBuf> {
    let dir = py.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut cands = Vec::new();
    if dir
        .file_name()
        .map_or(false, |n| n.eq_ignore_ascii_case("Scripts"))
    {
        if let Some(p) = dir.parent() {
            cands.push(p.join("Lib").join("site-packages"));
        }
    } else {
        cands.push(dir.join("Lib").join("site-packages"));
    }
    cands.into_iter().filter(|p| p.is_dir()).collect()
}

fn norm_key(s: &str) -> String {
    s.to_lowercase().replace('_', "-")
}
fn display_name(s: &str) -> String {
    s.replace('_', "-")
}

/// 纯 Rust 扫 dist-info/egg-info 目录名得包列表（无 python 启动）。
fn scan_installed(py: &Path) -> Vec<Package> {
    let mut out: BTreeMap<String, Package> = BTreeMap::new();
    for sp in site_packages_dirs(py) {
        let rd = match std::fs::read_dir(&sp) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            let stem = fname
                .strip_suffix(".dist-info")
                .or_else(|| fname.strip_suffix(".egg-info"));
            if let Some(stem) = stem {
                if let Some((n, v)) = stem.rsplit_once('-') {
                    if n.is_empty() {
                        continue;
                    }
                    let name = display_name(n);
                    out.entry(norm_key(n)).or_insert(Package {
                        name,
                        version: v.to_string(),
                    });
                }
            }
        }
    }
    out.into_values().collect()
}

pub fn list_packages(py: &Path) -> Result<Vec<Package>> {
    let fast = scan_installed(py);
    if !fast.is_empty() {
        return Ok(fast);
    }
    list_via_pip(py)
}

fn list_via_pip(py: &Path) -> Result<Vec<Package>> {
    let out = pip(py)
        .args(["list", "--format=json"])
        .output()
        .map_err(|e| PvmError::Config(format!("运行 pip 失败: {e}")))?;
    if !out.status.success() {
        return Err(PvmError::Config(format!(
            "pip list 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
        .map_err(|e| PvmError::Config(format!("解析 pip 输出失败: {e}")))?;
    Ok(v.into_iter()
        .filter_map(|p| {
            Some(Package {
                name: p.get("name")?.as_str()?.to_string(),
                version: p.get("version")?.as_str()?.to_string(),
            })
        })
        .collect())
}

// ---------- 纯 Rust：本地详情（读 METADATA） ----------

/// 纯 Rust 读取 dist-info/METADATA 得本地详情 + 反查 Required-by（不启动 python/网络）。
pub fn local_detail(py: &Path, name: &str) -> Result<PkgDetail> {
    let mut d = PkgDetail {
        name: name.to_string(),
        ..Default::default()
    };
    let target = norm_key(name);
    // (包名, 其 Requires-Dist 依赖名集合) —— 用于反查 required-by
    let mut graph: Vec<(String, Vec<String>)> = Vec::new();

    for sp in site_packages_dirs(py) {
        let rd = match std::fs::read_dir(&sp) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if !fname.ends_with(".dist-info") {
                continue;
            }
            let stem = fname.trim_end_matches(".dist-info");
            let pkgname = stem
                .rsplit_once('-')
                .map(|(n, _)| display_name(n))
                .unwrap_or_default();
            let text = match std::fs::read_to_string(e.path().join("METADATA")) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let reqs: Vec<String> = parse_requires(&text)
                .iter()
                .map(|r| dep_name(r))
                .filter(|x| !x.is_empty())
                .collect();
            if norm_key(&pkgname) == target {
                fill_from_metadata(&mut d, &text);
                d.location = sp.display().to_string();
                let mut rq = reqs.clone();
                rq.sort();
                rq.dedup();
                d.requires = rq;
            }
            graph.push((pkgname, reqs));
        }
    }

    let mut rb: Vec<String> = graph
        .into_iter()
        .filter(|(_, reqs)| reqs.iter().any(|r| norm_key(r) == target))
        .map(|(n, _)| n)
        .collect();
    rb.sort();
    rb.dedup();
    d.required_by = rb;
    Ok(d)
}

fn fill_from_metadata(d: &mut PkgDetail, text: &str) {
    for line in text.lines() {
        if line.is_empty() {
            break; // 头部结束，后面是正文 description
        }
        if let Some(v) = line.strip_prefix("Version:") {
            d.version = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Summary:") {
            d.summary = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Author:") {
            if d.author.is_empty() {
                d.author = v.trim().to_string();
            }
        } else if let Some(v) = line.strip_prefix("Author-email:") {
            if d.author.is_empty() {
                d.author = v.trim().to_string();
            }
        } else if let Some(v) = line.strip_prefix("License-Expression:") {
            d.license = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("License:") {
            if d.license.is_empty() {
                d.license = v.trim().to_string();
            }
        } else if let Some(v) = line.strip_prefix("Home-page:") {
            d.home_page = v.trim().to_string();
        }
    }
}

fn parse_requires(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("Requires-Dist:").map(|s| s.trim().to_string()))
        .filter(|s| !s.contains("extra ==")) // 跳过仅 extra 才需要的可选依赖
        .collect()
}

/// 从 "foo>=1.0; python_version<'3.9'" 提取裸包名 "foo"。
fn dep_name(req: &str) -> String {
    req.split(|c: char| {
        c == ';' || c == '(' || c == '=' || c == '<' || c == '>' || c == '!' || c == '~'
            || c == ' ' || c == '['
    })
    .next()
    .unwrap_or("")
    .trim()
    .to_string()
}

// ---------- PyPI（网络，异步补充） ----------

pub fn pypi_info(name: &str) -> Result<PypiInfo> {
    let url = format!("https://pypi.org/pypi/{name}/json");
    let resp = ureq::get(&url)
        .header("User-Agent", "pvm")
        .call()
        .map_err(|e| PvmError::Http(format!("PyPI 请求失败: {e}")))?;
    let mut resp = resp;
    let mut s = String::new();
    resp.body_mut()
        .as_reader()
        .read_to_string(&mut s)
        .map_err(|e| PvmError::Http(e.to_string()))?;
    let v: serde_json::Value =
        serde_json::from_str(&s).map_err(|e| PvmError::Http(format!("解析 PyPI 失败: {e}")))?;
    let info = v.get("info").cloned().unwrap_or_default();
    let gs = |k: &str| {
        info.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let mut project_urls = Vec::new();
    if let Some(obj) = info.get("project_urls").and_then(|x| x.as_object()) {
        for (k, val) in obj {
            if let Some(u) = val.as_str() {
                project_urls.push((k.clone(), u.to_string()));
            }
        }
    }
    let mut versions: Vec<String> = v
        .get("releases")
        .and_then(|x| x.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    versions.sort_by(|a, b| cmp_ver(b, a));
    Ok(PypiInfo {
        summary: gs("summary"),
        author: gs("author"),
        license: gs("license"),
        home_page: gs("home_page"),
        project_urls,
        available_versions: versions,
        readme: gs("description"),
    })
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

// ---------- pip 写操作（装/卸/升/过时） ----------

fn pip(py: &Path) -> Command {
    let mut c = Command::new(py);
    c.arg("-m").arg("pip").arg("--disable-pip-version-check");
    c.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
    no_window(&mut c);
    c
}

#[cfg(windows)]
fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
fn no_window(_c: &mut Command) {}

pub fn list_outdated(py: &Path) -> Result<Vec<Outdated>> {
    let out = pip(py)
        .args(["list", "--outdated", "--format=json"])
        .output()
        .map_err(|e| PvmError::Config(format!("运行 pip 失败: {e}")))?;
    if !out.status.success() {
        return Err(PvmError::Config(format!(
            "pip list --outdated 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap_or_default();
    Ok(v.into_iter()
        .filter_map(|p| {
            Some(Outdated {
                name: p.get("name")?.as_str()?.to_string(),
                version: p.get("version")?.as_str()?.to_string(),
                latest_version: p.get("latest_version")?.as_str()?.to_string(),
            })
        })
        .collect())
}

pub fn install(py: &Path, spec: &str, mirror_url: Option<&str>, upgrade: bool) -> Result<String> {
    let mut c = pip(py);
    c.arg("install").arg("--no-input");
    if upgrade {
        c.arg("--upgrade");
    }
    if let Some(m) = mirror_url {
        c.arg("-i").arg(m);
    }
    c.arg(spec);
    run_capture(c, "pip install")
}

pub fn uninstall(py: &Path, name: &str) -> Result<String> {
    let mut c = pip(py);
    c.args(["uninstall", "-y", name]);
    run_capture(c, "pip uninstall")
}

pub fn freeze(py: &Path) -> Result<String> {
    let out = pip(py)
        .args(["freeze"])
        .output()
        .map_err(|e| PvmError::Config(format!("运行 pip 失败: {e}")))?;
    if !out.status.success() {
        return Err(PvmError::Config(format!(
            "pip freeze 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn install_requirements(py: &Path, req_file: &Path, mirror_url: Option<&str>) -> Result<String> {
    if !req_file.is_file() {
        return Err(PvmError::Config(format!(
            "requirements 文件不存在: {}",
            req_file.display()
        )));
    }
    let mut c = pip(py);
    c.arg("install").arg("--no-input").arg("-r").arg(req_file);
    if let Some(m) = mirror_url {
        c.arg("-i").arg(m);
    }
    run_capture(c, "pip install -r")
}

fn run_capture(mut c: Command, label: &str) -> Result<String> {
    let out = c
        .output()
        .map_err(|e| PvmError::Config(format!("运行 {label} 失败: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(PvmError::Config(format!("{label} 失败:\n{stderr}")))
    }
}
