//! 自更新：查询 GitHub 上 pvm 的最新发行版，对比当前版本，下载并运行 NSIS 安装器。
//! 注意：api.github.com 在部分代理节点可能超时，fetch 带重试，失败由调用方静默处理。

use crate::download::{download_to, DownloadOpts};
use crate::error::{PvmError, Result};
use crate::paths::Paths;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "Eldon27232/pvm";

#[derive(serde::Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub notes: String,
    pub html_url: String,
    /// NSIS 安装器（setup.exe）资产直链；无则只能去 Releases 页手动下载。
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
}

/// 查询最新 release 并与 current 比较。网络失败返回 Err（调用方可静默忽略，不打扰启动）。
pub fn check_update(current: &str) -> Result<UpdateInfo> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let val = fetch_json(&url)?;

    let tag = val.get("tag_name").and_then(|x| x.as_str()).unwrap_or("");
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    let notes = val
        .get("body")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let html_url = val
        .get("html_url")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let (download_url, asset_name) = val
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|a| {
                let name = a.get("name").and_then(|x| x.as_str())?;
                let lname = name.to_lowercase();
                if lname.ends_with(".exe") && (lname.contains("setup") || lname.contains("install"))
                {
                    let dl = a.get("browser_download_url").and_then(|x| x.as_str())?;
                    Some((Some(dl.to_string()), Some(name.to_string())))
                } else {
                    None
                }
            })
        })
        .unwrap_or((None, None));

    Ok(UpdateInfo {
        has_update: !latest.is_empty() && version_gt(&latest, current),
        current: current.to_string(),
        latest,
        notes,
        html_url,
        download_url,
        asset_name,
    })
}

/// 下载安装器到 cache，返回本地路径。
pub fn download_installer(url: &str, asset_name: &str, paths: &Paths) -> Result<PathBuf> {
    std::fs::create_dir_all(paths.cache())?;
    // 资产名作文件名，避免路径穿越（只取末段文件名）。
    let safe = Path::new(asset_name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "pvm-setup.exe".to_string());
    let dest = paths.cache().join(safe);
    download_to(&DownloadOpts {
        url,
        dest: &dest,
        expect_sha256: None,
        quiet: true,
    })?;
    Ok(dest)
}

/// 启动安装器（独立进程）。NSIS 安装器自带界面，由用户继续操作；调用方一般随后退出本进程。
pub fn run_installer(path: &Path) -> Result<()> {
    Command::new(path)
        .spawn()
        .map_err(|e| PvmError::Http(format!("启动安装器失败: {e}")))?;
    Ok(())
}

fn fetch_json(url: &str) -> Result<serde_json::Value> {
    let token = std::env::var("GITHUB_TOKEN").ok();
    let mut last = String::new();
    for attempt in 0..3u32 {
        let mut req = crate::net::agent()
            .get(url)
            .header("User-Agent", "pvm")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &token {
            let auth = format!("Bearer {t}");
            req = req.header("Authorization", auth.as_str());
        }
        match req.call() {
            Ok(resp) => {
                let mut resp = resp;
                let mut s = String::new();
                resp.body_mut()
                    .as_reader()
                    .read_to_string(&mut s)
                    .map_err(|e| PvmError::Http(e.to_string()))?;
                return serde_json::from_str(&s)
                    .map_err(|e| PvmError::Http(format!("解析 GitHub 响应失败: {e}")));
            }
            Err(e) => {
                last = e.to_string();
                let lower = last.to_lowercase();
                let retriable = last.contains("502")
                    || last.contains("503")
                    || last.contains("504")
                    || lower.contains("timeout")
                    || lower.contains("timed out");
                if retriable && attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(400 * (attempt as u64 + 1)));
                    continue;
                }
                break;
            }
        }
    }
    Err(PvmError::Http(format!(
        "检查更新失败（GitHub API 不可达，可稍后重试或去 Releases 页手动查看）: {last}"
    )))
}

/// a > b ?（语义化版本比较，解析失败回退字符串比较）
fn version_gt(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(x), Ok(y)) => x > y,
        _ => a > b,
    }
}
