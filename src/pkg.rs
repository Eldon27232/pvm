//! Python 包管理：对任意 python.exe 执行 pip / importlib.metadata 操作。
//! 列包优先用 importlib.metadata（直读 dist-info，比启动 pip 快数倍），失败回退 pip。
//! 详情合并本地 pip show 与 PyPI JSON（元数据 / 可用版本 / README）。

use crate::error::{PvmError, Result};
use serde::Serialize;
use std::io::Read;
use std::path::Path;
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
    pub project_urls: Vec<(String, String)>,
    pub available_versions: Vec<String>,
    pub readme: String,
}

/// `python -m pip --disable-pip-version-check`，Windows 下隐藏子进程窗口。
fn pip(py: &Path) -> Command {
    let mut c = Command::new(py);
    c.arg("-m").arg("pip").arg("--disable-pip-version-check");
    no_window(&mut c);
    c
}

#[cfg(windows)]
fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn no_window(_c: &mut Command) {}

/// 用目标解释器跑一段内联 Python，返回 stdout。
fn run_py(py: &Path, script: &str) -> Result<String> {
    let mut c = Command::new(py);
    c.arg("-c").arg(script);
    no_window(&mut c);
    let out = c
        .output()
        .map_err(|e| PvmError::Config(format!("运行 python 失败: {e}")))?;
    if !out.status.success() {
        return Err(PvmError::Config(format!(
            "python 脚本失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 列出已装包：优先 importlib.metadata（快），失败回退 pip list。
pub fn list_packages(py: &Path) -> Result<Vec<Package>> {
    let script = r#"import json,importlib.metadata as m
out={}
for d in m.distributions():
    try:
        n=d.metadata["Name"]
    except Exception:
        n=None
    if not n:
        continue
    k=n.lower()
    if k not in out:
        out[k]={"name":n,"version":d.version or ""}
print(json.dumps(list(out.values())))"#;
    match run_py(py, script) {
        Ok(s) => match serde_json::from_str::<Vec<Package>>(s.trim()) {
            Ok(v) if !v.is_empty() => Ok(v),
            _ => list_via_pip(py),
        },
        Err(_) => list_via_pip(py),
    }
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

/// 合并本地 pip show 与 PyPI JSON 的丰富详情。PyPI 失败不致命（离线仍有本地信息）。
pub fn package_detail(py: &Path, name: &str) -> Result<PkgDetail> {
    let mut d = PkgDetail {
        name: name.to_string(),
        ..Default::default()
    };
    if let Ok(show) = show(py, name) {
        for line in show.lines() {
            if let Some(v) = line.strip_prefix("Version:") {
                d.version = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Summary:") {
                d.summary = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Location:") {
                d.location = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Requires:") {
                d.requires = split_csv(v);
            } else if let Some(v) = line.strip_prefix("Required-by:") {
                d.required_by = split_csv(v);
            } else if let Some(v) = line.strip_prefix("Author:") {
                d.author = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("License:") {
                d.license = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Home-page:") {
                d.home_page = v.trim().to_string();
            }
        }
    }
    if let Ok(meta) = pypi_meta(name) {
        if d.summary.is_empty() {
            d.summary = meta.summary;
        }
        if d.author.is_empty() {
            d.author = meta.author;
        }
        if d.license.is_empty() {
            d.license = meta.license;
        }
        if d.home_page.is_empty() {
            d.home_page = meta.home_page;
        }
        d.project_urls = meta.project_urls;
        d.available_versions = meta.versions;
        d.readme = meta.readme;
    }
    Ok(d)
}

struct PypiMeta {
    summary: String,
    author: String,
    license: String,
    home_page: String,
    project_urls: Vec<(String, String)>,
    versions: Vec<String>,
    readme: String,
}

fn pypi_meta(name: &str) -> Result<PypiMeta> {
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
    Ok(PypiMeta {
        summary: gs("summary"),
        author: gs("author"),
        license: gs("license"),
        home_page: gs("home_page"),
        project_urls,
        versions,
        readme: gs("description"),
    })
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// 安装或升级一个包。`upgrade=true` 加 `--upgrade`；`mirror_url` 提供则 `-i`。
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

pub fn show(py: &Path, name: &str) -> Result<String> {
    let out = pip(py)
        .args(["show", name])
        .output()
        .map_err(|e| PvmError::Config(format!("运行 pip 失败: {e}")))?;
    if !out.status.success() {
        return Err(PvmError::Config(format!(
            "pip show 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
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
