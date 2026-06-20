//! Python 包管理：对任意 python.exe 执行 pip 操作（list / outdated / install /
//! uninstall / upgrade / show / freeze / install -r）。作用对象可为系统 Python、
//! pvm 管理的版本或虚拟环境。

use crate::error::{PvmError, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
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

/// 构造一个 `python -m pip` 命令，并在 Windows 下隐藏子进程控制台窗口。
fn pip(py: &Path) -> Command {
    let mut c = Command::new(py);
    c.arg("-m").arg("pip");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

pub fn list_packages(py: &Path) -> Result<Vec<Package>> {
    let out = pip(py)
        .args(["list", "--format=json", "--disable-pip-version-check"])
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
        .args(["list", "--outdated", "--format=json", "--disable-pip-version-check"])
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

/// 安装或升级一个包。`upgrade=true` 加 `--upgrade`；`mirror_url` 提供则 `-i`。
pub fn install(py: &Path, spec: &str, mirror_url: Option<&str>, upgrade: bool) -> Result<String> {
    let mut c = pip(py);
    c.arg("install");
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
    let mut c = pip(py);
    c.arg("install").arg("-r").arg(req_file);
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
