//! Python 版本标识、来源、用户选择符解析。
//!
//! 命名关键点（SPEC §8）：
//! - canonical id 后缀用 `standalone` / `org`（磁盘）。
//! - CLI `--source` 取值用 `standalone` / `cpython`（`cpython` 即 python.org 官方）。
//! - freethreaded 变体在 id 与选择符中以 patch 后的 `t` 表示，如 `cpython-3.13.14t-standalone`。

use crate::error::{PvmError, Result};
use std::cmp::Ordering;

/// 解释器来源。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Standalone,
    Org,
}

impl Source {
    /// canonical id 中的后缀（磁盘标识）。
    pub fn id_suffix(self) -> &'static str {
        match self {
            Source::Standalone => "standalone",
            Source::Org => "org",
        }
    }

    /// CLI `--source` 的取值（`cpython` == python.org 官方）。
    pub fn cli_value(self) -> &'static str {
        match self {
            Source::Standalone => "standalone",
            Source::Org => "cpython",
        }
    }

    /// 同时接受 `cpython` 与 `org` 解析为 Org，便于 selector 的 `@org` 与 `--source cpython` 一致。
    pub fn from_cli(s: &str) -> Option<Self> {
        match s {
            "standalone" => Some(Self::Standalone),
            "cpython" | "org" => Some(Self::Org),
            _ => None,
        }
    }
}

/// 一个具体的、已解析到来源与三元版本号的 Python 版本。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PythonVersion {
    pub source: Source,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// standalone 的 free-threaded 变体（id 中 patch 后加 `t`）。
    pub freethreaded: bool,
}

impl PythonVersion {
    /// canonical id，唯一标识磁盘目录。
    /// 例：`cpython-3.12.7-standalone` / `cpython-3.12.7-org` / `cpython-3.13.14t-standalone`。
    pub fn canonical(&self) -> String {
        let t = if self.freethreaded { "t" } else { "" };
        format!(
            "cpython-{}.{}.{}{}-{}",
            self.major,
            self.minor,
            self.patch,
            t,
            self.source.id_suffix()
        )
    }

    /// 纯三元版本号字符串，例 `3.12.7`。
    pub fn xyz(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    /// 解析 canonical id（canonical() 的逆操作，对 `t` 后缀双向无损）。
    pub fn parse_canonical(id: &str) -> Result<Self> {
        let rest = id
            .strip_prefix("cpython-")
            .ok_or_else(|| PvmError::Config(format!("非法 canonical id: {id}")))?;
        let (verpart, suffix) = rest
            .rsplit_once('-')
            .ok_or_else(|| PvmError::Config(format!("非法 canonical id: {id}")))?;
        let source = match suffix {
            "standalone" => Source::Standalone,
            "org" => Source::Org,
            _ => return Err(PvmError::Config(format!("未知来源后缀: {suffix}"))),
        };
        let (vnum, freethreaded) = match verpart.strip_suffix('t') {
            Some(v) => (v, true),
            None => (verpart, false),
        };
        let (major, minor, patch) = parse_triple(vnum)?;
        Ok(Self {
            source,
            major,
            minor,
            patch,
            freethreaded,
        })
    }
}

impl PartialOrd for PythonVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PythonVersion {
    /// 先比 (major,minor,patch) 的 semver 序，再 freethreaded(false<true)，再来源后缀。
    /// 用于「部分版本取最高 patch」。
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            // 非 free-threaded 视为"更大"，使"取最高版本"默认偏好非 ft（ft 须显式 t 后缀 opt-in）
            .then(other.freethreaded.cmp(&self.freethreaded))
            .then(self.source.id_suffix().cmp(other.source.id_suffix()))
    }
}

/// 用户在 CLI / `.python-version` 中输入的版本选择符。
#[derive(Clone, Debug)]
pub enum VersionSelector {
    Exact {
        ver: (u32, u32, u32),
        source: Option<Source>,
        freethreaded: bool,
    },
    PartialMinor {
        major: u32,
        minor: u32,
        source: Option<Source>,
        freethreaded: bool,
    },
    PartialMajor {
        major: u32,
        source: Option<Source>,
    },
    Latest {
        source: Option<Source>,
    },
    Canonical(PythonVersion),
    System,
}

impl VersionSelector {
    /// 选择符显式指定的来源（来自 @source 或 canonical），未指定为 None。
    pub fn source(&self) -> Option<Source> {
        match self {
            VersionSelector::Exact { source, .. }
            | VersionSelector::PartialMinor { source, .. }
            | VersionSelector::PartialMajor { source, .. }
            | VersionSelector::Latest { source } => *source,
            VersionSelector::Canonical(v) => Some(v.source),
            VersionSelector::System => None,
        }
    }
    /// 选择符是否表达 free-threaded 意图（t 后缀 / canonical）。
    pub fn freethreaded(&self) -> bool {
        match self {
            VersionSelector::Exact { freethreaded, .. }
            | VersionSelector::PartialMinor { freethreaded, .. } => *freethreaded,
            VersionSelector::Canonical(v) => v.freethreaded,
            _ => false,
        }
    }
    pub fn is_system(&self) -> bool {
        matches!(self, VersionSelector::System)
    }
}

/// 解析用户输入：`3.12` / `3.12.7` / `3.12.7@org` / canonical / `latest` / `system` / `3.13t`。
pub fn parse_selector(s: &str) -> Result<VersionSelector> {
    let s = s.trim();
    if s.is_empty() {
        return Err(PvmError::Config("空版本选择符".into()));
    }
    if s.starts_with("cpython-") {
        return Ok(VersionSelector::Canonical(PythonVersion::parse_canonical(s)?));
    }
    if s.eq_ignore_ascii_case("system") {
        return Ok(VersionSelector::System);
    }

    // 拆分 @source
    let (vpart, source) = match s.split_once('@') {
        Some((v, src)) => {
            let so = Source::from_cli(src)
                .ok_or_else(|| PvmError::Config(format!("未知来源: {src}")))?;
            (v, Some(so))
        }
        None => (s, None),
    };

    if vpart.eq_ignore_ascii_case("latest") {
        return Ok(VersionSelector::Latest { source });
    }

    // free-threaded 后缀 t（仅在带小数点的版本号上有意义）
    let (vnum, freethreaded) = match vpart.strip_suffix(['t', 'T']) {
        Some(v) if v.contains('.') => (v, true),
        _ => (vpart, false),
    };

    let parts: Vec<&str> = vnum.split('.').collect();
    match parts.as_slice() {
        [a, b, c] => Ok(VersionSelector::Exact {
            ver: (pu(a)?, pu(b)?, pu(c)?),
            source,
            freethreaded,
        }),
        [a, b] => Ok(VersionSelector::PartialMinor {
            major: pu(a)?,
            minor: pu(b)?,
            source,
            freethreaded,
        }),
        [a] => Ok(VersionSelector::PartialMajor {
            major: pu(a)?,
            source,
        }),
        _ => Err(PvmError::Config(format!("无法解析版本选择符: {s}"))),
    }
}

fn parse_triple(vnum: &str) -> Result<(u32, u32, u32)> {
    let parts: Vec<&str> = vnum.split('.').collect();
    match parts.as_slice() {
        [a, b, c] => Ok((pu(a)?, pu(b)?, pu(c)?)),
        _ => Err(PvmError::Config(format!("非法版本号: {vnum}"))),
    }
}

fn pu(s: &str) -> Result<u32> {
    s.parse::<u32>()
        .map_err(|_| PvmError::Config(format!("非法版本号片段: {s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_roundtrip_freethreaded() {
        let v = PythonVersion {
            source: Source::Standalone,
            major: 3,
            minor: 13,
            patch: 14,
            freethreaded: true,
        };
        assert_eq!(v.canonical(), "cpython-3.13.14t-standalone");
        assert_eq!(PythonVersion::parse_canonical(&v.canonical()).unwrap(), v);
    }

    #[test]
    fn canonical_roundtrip_org() {
        let v = PythonVersion {
            source: Source::Org,
            major: 3,
            minor: 12,
            patch: 7,
            freethreaded: false,
        };
        assert_eq!(v.canonical(), "cpython-3.12.7-org");
        assert_eq!(PythonVersion::parse_canonical("cpython-3.12.7-org").unwrap(), v);
    }

    #[test]
    fn selector_variants() {
        assert!(matches!(
            parse_selector("3.12").unwrap(),
            VersionSelector::PartialMinor { .. }
        ));
        assert!(matches!(
            parse_selector("3.12.7@org").unwrap(),
            VersionSelector::Exact {
                source: Some(Source::Org),
                ..
            }
        ));
        assert!(matches!(
            parse_selector("3.13.2t").unwrap(),
            VersionSelector::Exact {
                freethreaded: true,
                ..
            }
        ));
        assert!(matches!(
            parse_selector("latest@standalone").unwrap(),
            VersionSelector::Latest {
                source: Some(Source::Standalone)
            }
        ));
        assert!(matches!(parse_selector("system").unwrap(), VersionSelector::System));
        assert!(matches!(parse_selector("3").unwrap(), VersionSelector::PartialMajor { .. }));
    }

    #[test]
    fn ord_picks_highest_patch() {
        let a = PythonVersion { source: Source::Standalone, major: 3, minor: 12, patch: 1, freethreaded: false };
        let b = PythonVersion { source: Source::Standalone, major: 3, minor: 12, patch: 9, freethreaded: false };
        assert!(b > a);
    }

    #[test]
    fn ord_prefers_non_freethreaded_at_same_version() {
        let ft = PythonVersion { source: Source::Standalone, major: 3, minor: 13, patch: 2, freethreaded: true };
        let non = PythonVersion { source: Source::Standalone, major: 3, minor: 13, patch: 2, freethreaded: false };
        assert!(non > ft); // 同版本时非 ft 视为更大，取最高默认得非 ft
    }
}
