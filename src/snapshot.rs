//! 环境快照 / 克隆：把某解释器的已装包集（pip freeze）保存为命名快照，
//! 之后可在另一解释器重建（克隆）。快照存于 root\snapshots\<name>.json。

use crate::error::{PvmError, Result};
use crate::pkg;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub py_label: String,
    pub created_at: String,
    pub packages: Vec<String>,
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// freeze 目标解释器并保存为快照。
pub fn save(py: &Path, name: &str, py_label: &str, snap_dir: &Path) -> Result<()> {
    if name.trim().is_empty() {
        return Err(PvmError::Config("快照名不能为空".into()));
    }
    let freeze = pkg::freeze(py)?;
    let packages: Vec<String> = freeze
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("-e "))
        .collect();
    let snap = Snapshot {
        name: name.to_string(),
        py_label: py_label.to_string(),
        created_at: chrono::Local::now().to_rfc3339(),
        packages,
    };
    std::fs::create_dir_all(snap_dir)?;
    let text = serde_json::to_string_pretty(&snap).map_err(|e| PvmError::Config(e.to_string()))?;
    std::fs::write(snap_dir.join(format!("{}.json", sanitize(name))), text)?;
    Ok(())
}

pub fn list(snap_dir: &Path) -> Vec<Snapshot> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(snap_dir) {
        for e in rd.flatten() {
            if e.path().extension().map_or(false, |x| x == "json") {
                if let Ok(text) = std::fs::read_to_string(e.path()) {
                    if let Ok(s) = serde_json::from_str::<Snapshot>(&text) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

pub fn load(snap_dir: &Path, name: &str) -> Result<Snapshot> {
    let path = snap_dir.join(format!("{}.json", sanitize(name)));
    let text =
        std::fs::read_to_string(&path).map_err(|_| PvmError::Config(format!("快照不存在: {name}")))?;
    serde_json::from_str(&text).map_err(|e| PvmError::Config(format!("解析快照失败: {e}")))
}

pub fn delete(snap_dir: &Path, name: &str) -> Result<()> {
    let path = snap_dir.join(format!("{}.json", sanitize(name)));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// 克隆：把快照包集装到目标解释器（写临时 requirements + pip install -r）。
pub fn apply(py: &Path, snap: &Snapshot, mirror_url: Option<&str>, work_dir: &Path) -> Result<String> {
    std::fs::create_dir_all(work_dir)?;
    let req = work_dir.join(format!(".snap-{}.txt", sanitize(&snap.name)));
    std::fs::write(&req, snap.packages.join("\n"))?;
    let r = pkg::install_requirements(py, &req, mirror_url);
    let _ = std::fs::remove_file(&req);
    r
}
