//! 版本解析：生效版本（shim 与 `pvm current` 共用）与 selector → 已安装版本。
//!
//! 优先级（resolve_effective）：PVM_VERSION > 向上找 .python-version > root\version > Err。

use crate::config::Config;
use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::version::{parse_selector, PythonVersion, Source, VersionSelector};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ResolvedFrom {
    Shell,
    Local(PathBuf),
    Global,
}

pub struct Effective {
    pub version: PythonVersion,
    pub from: ResolvedFrom,
    pub interpreter_dir: PathBuf,
}

/// 列出所有已安装版本（扫 versions 目录，要求解释器真实存在）。
pub fn list_installed(paths: &Paths) -> Result<Vec<PythonVersion>> {
    let dir = paths.versions();
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Ok(v) = PythonVersion::parse_canonical(&name) {
            if paths.python_exe(&v).exists() {
                out.push(v);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// 某具体版本是否已安装。
pub fn is_installed(v: &PythonVersion, paths: &Paths) -> bool {
    paths.python_exe(v).exists()
}

/// selector → 已安装的具体版本。
///
/// 语义：PreferDefault —— 当同一 (x.y.z, freethreaded) 在多来源并存且 selector 未指定来源时，
/// 优先选 `default_source`；若 default_source 不在候选中则返回 Ambiguous。
/// （命令层 global/local/uninstall 如需严格歧义提示，可在调用前自查候选数，见 SPEC §8.2）
pub fn resolve_installed(
    sel: &VersionSelector,
    default_source: Source,
    paths: &Paths,
) -> Result<PythonVersion> {
    if sel.is_system() {
        return Err(PvmError::Usage(
            "system 别名本期未实现（指 PATH 上 pvm 之外的 Python）".into(),
        ));
    }
    let mut candidates: Vec<PythonVersion> = list_installed(paths)?
        .into_iter()
        .filter(|v| selector_matches(sel, v))
        .collect();
    if candidates.is_empty() {
        return Err(PvmError::NotInstalled(selector_desc(sel)));
    }
    candidates.sort();

    if selector_source(sel).is_none() {
        let top = candidates.last().unwrap().clone();
        let same: Vec<&PythonVersion> = candidates
            .iter()
            .filter(|v| {
                v.major == top.major
                    && v.minor == top.minor
                    && v.patch == top.patch
                    && v.freethreaded == top.freethreaded
            })
            .collect();
        if same.len() > 1 {
            if let Some(v) = same.iter().find(|v| v.source == default_source) {
                return Ok((*v).clone());
            }
            let list = same
                .iter()
                .map(|v| v.canonical())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PvmError::Ambiguous(list));
        }
        return Ok(top);
    }
    Ok(candidates.last().unwrap().clone())
}

/// 生效版本解析（shim 与 CLI 共用）。
pub fn resolve_effective(cwd: &Path, paths: &Paths) -> Result<Effective> {
    let config = Config::load(paths)?;
    let default_source = config.default_source_resolved();

    if let Ok(val) = std::env::var("PVM_VERSION") {
        let val = val.trim();
        if !val.is_empty() {
            return finalize(val, ResolvedFrom::Shell, default_source, paths);
        }
    }

    if !config.disable_auto_switch {
        if let Some((file, raw)) = find_dotfile_upwards(cwd)? {
            return finalize(raw.trim(), ResolvedFrom::Local(file), default_source, paths);
        }
    }

    let gf = paths.global_version_file();
    if gf.exists() {
        let raw = std::fs::read_to_string(&gf)?;
        let raw = raw.trim();
        if !raw.is_empty() {
            return finalize(raw, ResolvedFrom::Global, default_source, paths);
        }
    }

    Err(PvmError::NoVersionConfigured)
}

fn finalize(
    req: &str,
    from: ResolvedFrom,
    default_source: Source,
    paths: &Paths,
) -> Result<Effective> {
    let sel = parse_selector(req)?;
    let v = resolve_installed(&sel, default_source, paths)?;
    let interpreter_dir = paths.version_dir(&v);
    Ok(Effective {
        version: v,
        from,
        interpreter_dir,
    })
}

/// 从 start 逐级向上查找 .python-version，返回 (文件路径, 原始内容)。
pub fn find_dotfile_upwards(start: &Path) -> Result<Option<(PathBuf, String)>> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(".python-version");
        if candidate.is_file() {
            let content = std::fs::read_to_string(&candidate)?;
            return Ok(Some((candidate, content)));
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => return Ok(None),
        }
    }
}

fn selector_source(sel: &VersionSelector) -> Option<Source> {
    match sel {
        VersionSelector::Exact { source, .. }
        | VersionSelector::PartialMinor { source, .. }
        | VersionSelector::PartialMajor { source, .. }
        | VersionSelector::Latest { source } => *source,
        VersionSelector::Canonical(v) => Some(v.source),
        VersionSelector::System => None,
    }
}

fn selector_matches(sel: &VersionSelector, v: &PythonVersion) -> bool {
    let src_ok = |s: &Option<Source>| s.map_or(true, |s| s == v.source);
    match sel {
        VersionSelector::Exact {
            ver,
            source,
            freethreaded,
        } => {
            v.major == ver.0
                && v.minor == ver.1
                && v.patch == ver.2
                && v.freethreaded == *freethreaded
                && src_ok(source)
        }
        VersionSelector::PartialMinor {
            major,
            minor,
            source,
            freethreaded,
        } => {
            v.major == *major
                && v.minor == *minor
                && v.freethreaded == *freethreaded
                && src_ok(source)
        }
        VersionSelector::PartialMajor { major, source } => v.major == *major && src_ok(source),
        VersionSelector::Latest { source } => src_ok(source),
        VersionSelector::Canonical(want) => v == want,
        VersionSelector::System => false,
    }
}

fn selector_desc(sel: &VersionSelector) -> String {
    match sel {
        VersionSelector::Exact { ver, .. } => format!("{}.{}.{}", ver.0, ver.1, ver.2),
        VersionSelector::PartialMinor { major, minor, .. } => format!("{major}.{minor}"),
        VersionSelector::PartialMajor { major, .. } => format!("{major}"),
        VersionSelector::Latest { .. } => "latest".into(),
        VersionSelector::Canonical(v) => v.canonical(),
        VersionSelector::System => "system".into(),
    }
}
