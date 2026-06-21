//! 虚拟环境：基于选定版本 `python -m venv` 创建，集中式管理 + 元数据 + 激活提示。

use crate::config::Config;
use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::pip::{self, Scope};
use crate::resolve::{resolve_effective, resolve_installed};
use crate::version::{parse_selector, PythonVersion};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 给子进程加 CREATE_NO_WINDOW，避免 release GUI（无 console）调用外部命令时闪现控制台窗口。
#[cfg(windows)]
fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
fn no_window(_c: &mut Command) {}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct VenvMeta {
    pub name: String,
    pub python_version: String,
    pub source: String,
    pub base_prefix: PathBuf,
    pub created_at: String,
}

pub struct VenvCreateOpts<'a> {
    pub name: &'a str,
    pub py_selector: Option<&'a str>,
    /// 直接指定解释器（任意 python.exe，包含系统已装 Python）；设置后优先于 py_selector。
    pub base_exe: Option<&'a Path>,
    /// base_exe 模式下写入元数据的版本标签（仅显示用）。
    pub base_label: Option<&'a str>,
    pub in_project: bool,
    pub path: Option<&'a Path>,
    pub clear: bool,
    pub without_pip: bool,
    pub system_site_packages: bool,
    pub mirror: Option<&'a str>,
}

pub fn venv_create(opts: &VenvCreateOpts, paths: &Paths) -> Result<PathBuf> {
    // 解释器解析：base_exe（任意解释器，含系统 Python）优先；否则按 selector / 生效版本解析 pvm 管理的版本。
    let (py, version_label, source_label, base_prefix): (PathBuf, String, String, PathBuf) =
        if let Some(exe) = opts.base_exe {
            if !exe.exists() {
                return Err(PvmError::Config(format!("解释器不存在: {}", exe.display())));
            }
            let base = exe.parent().map(Path::to_path_buf).unwrap_or_default();
            let label = opts
                .base_label
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("system")
                .to_string();
            (exe.to_path_buf(), label, "system".to_string(), base)
        } else {
            let config = Config::load(paths)?;
            let default_source = config.default_source_resolved();
            let v: PythonVersion = match opts.py_selector {
                Some(sel) => resolve_installed(&parse_selector(sel)?, default_source, paths)?,
                None => resolve_effective(&std::env::current_dir()?, paths)?.version,
            };
            let py = paths.python_exe(&v);
            if !py.exists() {
                return Err(PvmError::NotInstalled(v.canonical()));
            }
            (
                py,
                v.canonical(),
                v.source.id_suffix().to_string(),
                paths.version_dir(&v),
            )
        };

    let target: PathBuf = if opts.in_project {
        std::env::current_dir()?.join(".venv")
    } else if let Some(p) = opts.path {
        p.to_path_buf()
    } else {
        paths.venvs().join(opts.name)
    };

    if target.exists() && !opts.clear {
        let nonempty = std::fs::read_dir(&target)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if nonempty {
            return Err(PvmError::Config(format!(
                "目标已存在且非空: {}（用 --clear 覆盖）",
                target.display()
            )));
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut cmd = Command::new(&py);
    cmd.arg("-m").arg("venv");
    if opts.clear {
        cmd.arg("--clear");
    }
    if opts.without_pip {
        cmd.arg("--without-pip");
    }
    if opts.system_site_packages {
        cmd.arg("--system-site-packages");
    }
    cmd.arg(&target);
    no_window(&mut cmd); // GUI 无 console，避免创建 venv 时闪现 cmd 窗口
    let status = cmd
        .status()
        .map_err(|e| PvmError::Config(format!("启动 python -m venv 失败: {e}")))?;
    if !status.success() {
        return Err(PvmError::Config(format!(
            "python -m venv 失败（退出码 {:?}）",
            status.code()
        )));
    }

    if !opts.in_project {
        let meta = VenvMeta {
            name: opts.name.to_string(),
            python_version: version_label,
            source: source_label,
            base_prefix,
            created_at: chrono::Local::now().to_rfc3339(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&meta) {
            std::fs::write(target.join(".pvm-venv.json"), text)?;
        }
    }

    if let Some(m) = opts.mirror {
        pip::pip_mirror_set(m, Scope::Venv, Some(&target), false, paths)?;
    }

    Ok(target)
}

pub fn venv_list(paths: &Paths) -> Result<Vec<VenvMeta>> {
    let dir = paths.venvs();
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        let meta_path = e.path().join(".pvm-venv.json");
        if let Ok(text) = std::fs::read_to_string(&meta_path) {
            if let Ok(meta) = serde_json::from_str::<VenvMeta>(&text) {
                out.push(meta);
                continue;
            }
        }
        out.push(VenvMeta {
            name: e.file_name().to_string_lossy().to_string(),
            python_version: "(未知)".into(),
            source: "(未知)".into(),
            base_prefix: PathBuf::new(),
            created_at: String::new(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn venv_remove(name: &str, paths: &Paths, _yes: bool) -> Result<()> {
    let target = paths.venvs().join(name);
    if !target.exists() {
        return Err(PvmError::Config(format!("venv 不存在: {name}")));
    }
    std::fs::remove_dir_all(&target)?;
    Ok(())
}

pub fn venv_path(name: &str, paths: &Paths) -> PathBuf {
    paths.venvs().join(name)
}

/// 返回 venv 激活提示文本（不打印，供 CLI/GUI 各自决定输出方式）。
pub fn activation_hint(venv: &Path) -> String {
    let s = venv.join("Scripts").display().to_string();
    let fwd = s.replace('\\', "/");
    format!(
        "激活方式：\n  PowerShell : & '{s}\\Activate.ps1'\n  cmd.exe    : {s}\\activate.bat\n  Git Bash   : source '{fwd}/activate'\n  （PowerShell 若被执行策略拦截：Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass）"
    )
}
