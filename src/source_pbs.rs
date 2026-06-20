//! python-build-standalone 来源：枚举 GitHub Releases 资产、下载解压安装。

use crate::archive::{extract, ArchiveKind};
use crate::download::{download_to, DownloadOpts};
use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::version::PythonVersion;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

pub const PBS_REPO: &str = "astral-sh/python-build-standalone";
pub const TARGET_TRIPLE: &str = "x86_64-pc-windows-msvc";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PbsFlavor {
    InstallOnly,
    InstallOnlyStripped,
    PgoFull,
}

impl PbsFlavor {
    pub fn suffix(self) -> &'static str {
        match self {
            PbsFlavor::InstallOnly => "install_only.tar.gz",
            PbsFlavor::InstallOnlyStripped => "install_only_stripped.tar.gz",
            PbsFlavor::PgoFull => "pgo-full.tar.zst",
        }
    }
    pub fn is_zstd(self) -> bool {
        matches!(self, PbsFlavor::PgoFull)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PbsAsset {
    pub python_version: semver::Version,
    pub release_date: String,
    pub freethreaded: bool,
    pub flavor: PbsFlavor,
    pub download_url: String,
    pub size: u64,
    pub sha256: Option<String>,
    pub file_name: String,
}

/// 枚举可安装资产（分页 + 文件缓存）。flavor/freethreaded 过滤；按版本降序、同版本保最新日期。
pub fn list_pbs_assets(
    token: Option<&str>,
    flavor: PbsFlavor,
    freethreaded: bool,
    paths: &Paths,
    refresh: bool,
) -> Result<Vec<PbsAsset>> {
    let cache = paths.cache().join("pbs-index.json");
    if !refresh {
        if let Some(all) = load_cache(&cache) {
            return Ok(filter_assets(all, flavor, freethreaded));
        }
    }
    let all = fetch_all_assets(token)?;
    save_cache(&cache, &all);
    Ok(filter_assets(all, flavor, freethreaded))
}

fn filter_assets(all: Vec<PbsAsset>, flavor: PbsFlavor, freethreaded: bool) -> Vec<PbsAsset> {
    let mut v: Vec<PbsAsset> = all
        .into_iter()
        .filter(|a| a.flavor == flavor && a.freethreaded == freethreaded)
        .collect();
    // 同 python_version 保最新 release_date：先按 (版本升, 日期降) 排，dedup 保留首个
    v.sort_by(|a, b| {
        a.python_version
            .cmp(&b.python_version)
            .then(b.release_date.cmp(&a.release_date))
    });
    v.dedup_by(|a, b| a.python_version == b.python_version);
    v.sort_by(|a, b| b.python_version.cmp(&a.python_version));
    v
}

fn load_cache(path: &Path) -> Option<Vec<PbsAsset>> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_cache(path: &Path, all: &[PbsAsset]) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Ok(text) = serde_json::to_string(all) {
        let _ = std::fs::write(path, text);
    }
}

fn fetch_all_assets(token: Option<&str>) -> Result<Vec<PbsAsset>> {
    let re = regex::Regex::new(
        r"^cpython-(\d+\.\d+\.\d+)\+(\d{8})-x86_64-pc-windows-msvc-(.+)\.tar\.(?:gz|zst)$",
    )
    .expect("PBS 资产正则应当合法");
    let mut out = Vec::new();
    // PBS 每个日期 release 含当时全部 Python 版本；取近几页即可覆盖近期所有 minor 线
    let max_pages = 3u32;
    for page in 1..=max_pages {
        let releases = fetch_page(token, page)?;
        if releases.is_empty() {
            break;
        }
        for rel in &releases {
            if let Some(assets) = rel.get("assets").and_then(|a| a.as_array()) {
                for a in assets {
                    let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
                    if let Some(asset) = parse_asset(name, a, &re) {
                        out.push(asset);
                    }
                }
            }
        }
    }
    Ok(out)
}

fn fetch_page(token: Option<&str>, page: u32) -> Result<Vec<serde_json::Value>> {
    let url = format!("https://api.github.com/repos/{PBS_REPO}/releases?per_page=100&page={page}");
    let auth = token.map(|t| format!("Bearer {t}"));
    // api.github.com 经部分代理节点偶发 5xx/超时，重试几次提升成功率
    let mut last = String::new();
    for attempt in 0..4u32 {
        let mut req2 = crate::net::agent()
            .get(&url)
            .header("User-Agent", "pvm")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(a) = &auth {
            req2 = req2.header("Authorization", a.as_str());
        }
        match req2.call() {
            Ok(resp) => {
                let mut resp = resp;
                let mut s = String::new();
                resp.body_mut()
                    .as_reader()
                    .read_to_string(&mut s)
                    .map_err(|e| PvmError::Http(e.to_string()))?;
                let val: serde_json::Value = serde_json::from_str(&s)
                    .map_err(|e| PvmError::Http(format!("解析 GitHub 响应失败: {e}")))?;
                return match val {
                    serde_json::Value::Array(a) => Ok(a),
                    _ => Ok(Vec::new()),
                };
            }
            Err(e) => {
                let msg = e.to_string();
                let lower = msg.to_lowercase();
                if msg.contains("403") || msg.contains("429") || lower.contains("rate") {
                    return Err(PvmError::RateLimited {
                        reset: "稍后重试，或设置 GITHUB_TOKEN 提升限额".into(),
                    });
                }
                let retriable = msg.contains("504")
                    || msg.contains("502")
                    || msg.contains("503")
                    || lower.contains("timeout")
                    || lower.contains("timed out");
                last = msg;
                if retriable && attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1)));
                    continue;
                }
                break;
            }
        }
    }
    Err(PvmError::Http(format!(
        "GitHub API 请求失败（已重试；常见于代理对 api.github.com 的节点超时，可改用 --source cpython 或设 GITHUB_TOKEN）: {last}"
    )))
}

fn parse_asset(name: &str, a: &serde_json::Value, re: &regex::Regex) -> Option<PbsAsset> {
    let caps = re.captures(name)?;
    let ver = semver::Version::parse(&caps[1]).ok()?;
    let date = caps[2].to_string();
    let rest = &caps[3];
    let freethreaded = rest.contains("freethreaded");
    let flavor = if rest.ends_with("install_only_stripped") {
        PbsFlavor::InstallOnlyStripped
    } else if rest.ends_with("install_only") {
        PbsFlavor::InstallOnly
    } else if rest.contains("pgo-full") {
        PbsFlavor::PgoFull
    } else {
        return None;
    };
    let url = a
        .get("browser_download_url")
        .and_then(|x| x.as_str())?
        .to_string();
    let size = a.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
    let sha256 = a
        .get("digest")
        .and_then(|x| x.as_str())
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(|s| s.to_string());
    Some(PbsAsset {
        python_version: ver,
        release_date: date,
        freethreaded,
        flavor,
        download_url: url,
        size,
        sha256,
        file_name: name.to_string(),
    })
}

/// 下载 → 解压临时目录 → 按 flavor 校验解释器子路径 → 原子 rename 到 version_dir。
pub fn install_pbs(asset: &PbsAsset, v: &PythonVersion, paths: &Paths, no_verify: bool) -> Result<()> {
    std::fs::create_dir_all(paths.cache())?;
    std::fs::create_dir_all(paths.versions())?;

    let file = paths.cache().join(&asset.file_name);
    let sha = if no_verify { None } else { asset.sha256.as_deref() };
    download_to(&DownloadOpts {
        url: &asset.download_url,
        dest: &file,
        expect_sha256: sha,
        quiet: false,
    })?;

    let tmp = paths.versions().join(format!(".tmp-{}", v.canonical()));
    let regrouped = paths.versions().join(format!(".tmp2-{}", v.canonical()));
    let result = extract_and_place(asset, v, paths, &file, &tmp, &regrouped);
    // 任何失败路径都清理临时目录，避免残留累积占用磁盘
    if result.is_err() {
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::remove_dir_all(&regrouped).ok();
    }
    result
}

/// 同 install_pbs，但用多线程分块下载并通过 on_progress(已下载, 总量) 回调进度（GUI 用）。
pub fn install_pbs_progress(
    asset: &PbsAsset,
    v: &PythonVersion,
    paths: &Paths,
    no_verify: bool,
    threads: usize,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    std::fs::create_dir_all(paths.cache())?;
    std::fs::create_dir_all(paths.versions())?;

    let file = paths.cache().join(&asset.file_name);
    let sha = if no_verify { None } else { asset.sha256.as_deref() };
    crate::download::download_parallel(&asset.download_url, &file, sha, threads, on_progress)?;

    let tmp = paths.versions().join(format!(".tmp-{}", v.canonical()));
    let regrouped = paths.versions().join(format!(".tmp2-{}", v.canonical()));
    let result = extract_and_place(asset, v, paths, &file, &tmp, &regrouped);
    if result.is_err() {
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::remove_dir_all(&regrouped).ok();
    }
    result
}

fn extract_and_place(
    asset: &PbsAsset,
    v: &PythonVersion,
    paths: &Paths,
    file: &Path,
    tmp: &Path,
    regrouped: &Path,
) -> Result<()> {
    if tmp.exists() {
        std::fs::remove_dir_all(tmp)?;
    }
    let kind = if asset.flavor.is_zstd() {
        ArchiveKind::TarZst
    } else {
        ArchiveKind::TarGz
    };
    extract(file, tmp, kind)?;

    // 校验解释器存在
    let interp = match asset.flavor {
        PbsFlavor::PgoFull => tmp.join("python").join("install").join("python.exe"),
        _ => tmp.join("python").join("python.exe"),
    };
    if !interp.exists() {
        return Err(PvmError::Archive(format!(
            "解压后未找到解释器: {}",
            interp.display()
        )));
    }

    let version_dir = paths.version_dir(v);
    if matches!(asset.flavor, PbsFlavor::PgoFull) {
        // pgo-full 顶层为 python\install\，提升一层使布局与 install_only 一致（python\python.exe）
        if regrouped.exists() {
            std::fs::remove_dir_all(regrouped)?;
        }
        std::fs::create_dir_all(regrouped)?;
        std::fs::rename(tmp.join("python").join("install"), regrouped.join("python"))?;
        std::fs::remove_dir_all(tmp).ok();
        finalize_install(regrouped, &version_dir)?;
    } else {
        finalize_install(tmp, &version_dir)?;
    }
    Ok(())
}

fn finalize_install(tmp: &Path, version_dir: &Path) -> Result<()> {
    if version_dir.exists() {
        std::fs::remove_dir_all(version_dir)?;
    }
    if let Some(parent) = version_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(tmp, version_dir)?;
    Ok(())
}
