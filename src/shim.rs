//! shim 子系统：rehash（生成/清理转发器）与 run_shim（按 .python-version 解析并转发）。

use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::resolve::resolve_effective;
use crate::version::PythonVersion;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 为核心入口（python/pythonw/pip/pip3）+ 生效版本与各 venv 的 Scripts 入口生成 shim，
/// 并清理 shims 目录中的旧 exe。shim 模板取自 root\bin\pvm-shim(.exe)/pvm-shimw.exe。
pub fn rehash(p: &Paths) -> Result<()> {
    let shims = p.shims();
    std::fs::create_dir_all(&shims)?;

    let shim_console = p.bin().join("pvm-shim.exe");
    let shim_gui = p.bin().join("pvm-shimw.exe");

    // 清理旧 shim（全部 .exe），再重建——shim 数量可控，简单稳妥
    for e in std::fs::read_dir(&shims)? {
        let e = e?;
        if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
            std::fs::remove_file(e.path()).ok();
        }
    }

    let core = [
        ("python.exe", shim_console.as_path()),
        ("pythonw.exe", shim_gui.as_path()),
        ("pip.exe", shim_console.as_path()),
        ("pip3.exe", shim_console.as_path()),
    ];
    for (name, tmpl) in core {
        copy_shim(tmpl, &shims.join(name))?;
    }

    // 生效版本（global/local/shell）的 Scripts 入口
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(eff) = resolve_effective(&cwd, p) {
            rehash_scripts(&p.scripts_dir(&eff.version), &shims, &shim_console)?;
        }
    }

    // 各集中式 venv 的 Scripts 入口
    if let Ok(venvs) = crate::venv::venv_list(p) {
        for vm in venvs {
            let scripts = p.venvs().join(&vm.name).join("Scripts");
            rehash_scripts(&scripts, &shims, &shim_console)?;
        }
    }

    Ok(())
}

fn rehash_scripts(scripts: &Path, shims: &Path, tmpl: &Path) -> Result<()> {
    if !scripts.exists() {
        return Ok(());
    }
    for e in std::fs::read_dir(scripts)? {
        let e = e?;
        let path = e.path();
        if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
            if let Some(name) = path.file_name() {
                let dst = shims.join(name);
                if !dst.exists() {
                    copy_shim(tmpl, &dst)?;
                }
            }
        }
    }
    Ok(())
}

fn copy_shim(tmpl: &Path, dst: &Path) -> Result<()> {
    if !tmpl.exists() {
        return Err(PvmError::Config(format!(
            "shim 模板缺失: {}（请运行 pvm init 或确保 pvm-shim.exe 已就位）",
            tmpl.display()
        )));
    }
    std::fs::copy(tmpl, dst)?;
    Ok(())
}

/// shim 入口：解析生效版本，转发到真实解释器/脚本，透传参数与退出码。
pub fn run_shim() -> ! {
    std::process::exit(run_shim_inner());
}

fn run_shim_inner() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("pvm-shim: 无法获取自身路径: {e}");
            return 127;
        }
    };
    let stem = exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let paths = match Paths::discover(None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pvm: {e}");
            return 127;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let eff = match resolve_effective(&cwd, &paths) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("pvm: {e}");
            return 127;
        }
    };

    let target = locate_real_exe(&paths, &eff.version, &stem);
    let target = match target {
        Some(t) if t.exists() => t,
        _ => {
            eprintln!("pvm: 入口 {stem} 不存在于 {}", eff.version.canonical());
            return 127;
        }
    };

    #[cfg(windows)]
    unsafe {
        crate::winpath::install_noop_ctrl_handler();
    }

    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // 用原始 target 启动，不加 \\?\ 前缀——前缀会污染子进程 sys.executable（影响 pip/venv/再 spawn）
    match Command::new(&target).args(&args).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("pvm: 启动 {} 失败: {e}", target.display());
            126
        }
    }
}

fn locate_real_exe(paths: &Paths, v: &PythonVersion, stem: &str) -> Option<PathBuf> {
    match stem {
        "python" => Some(paths.python_exe(v)),
        "pythonw" => Some(paths.pythonw_exe(v)),
        other => Some(paths.scripts_dir(v).join(format!("{other}.exe"))),
    }
}
