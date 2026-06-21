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

/// 迁移：pvm 不再用 shim 接管 `python`。移除用户 PATH 里的 shims 目录项，并删除 shims 目录。
/// 调用后系统 `python` 完全恢复（不再经 pvm 转发）。可重复调用。
pub fn cleanup_legacy(p: &Paths) -> Result<()> {
    let shims = p.shims();
    #[cfg(windows)]
    {
        let s = shims.to_string_lossy().to_string();
        crate::winpath::remove_shims_from_user_path(&s)?;
    }
    if shims.exists() {
        std::fs::remove_dir_all(&shims).ok();
    }
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
    // 优先 pvm 生效版本；无生效版本（或该入口缺失）时回退到 PATH 中的系统可执行，
    // 避免 shim 抢占 python/pip 后拦截其它依赖系统 Python 的项目（兼容性关键）。
    let eff = resolve_effective(&cwd, &paths).ok();
    let pvm_target = eff
        .as_ref()
        .and_then(|e| locate_real_exe(&paths, &e.version, &stem))
        .filter(|t| t.exists());

    let target = match pvm_target.or_else(|| fallback_in_path(&stem, &paths.shims())) {
        Some(t) => t,
        None => {
            match &eff {
                Some(e) => eprintln!(
                    "pvm: 入口 {stem} 不存在于 {}，且 PATH 中也未找到系统 {stem}",
                    e.version.canonical()
                ),
                None => eprintln!(
                    "pvm: 未设置 Python 版本，且 PATH 中未找到系统 {stem}（可 `pvm global <版本>` 设全局，或把系统 Python 加入 PATH）"
                ),
            }
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

/// 无 pvm 生效版本时的兼容回退：在 PATH 中查找真实的 `<stem>.exe`，
/// 跳过 pvm 自己的 shims 目录（防递归）与 WindowsApps 的 Microsoft Store 占位 stub。
fn fallback_in_path(stem: &str, shims_dir: &Path) -> Option<PathBuf> {
    let exe_name = format!("{stem}.exe");
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() || same_dir(&dir, shims_dir) {
            continue;
        }
        // 跳过 Microsoft Store 的假 python（运行会弹商店，不是真解释器）
        if dir.to_string_lossy().to_lowercase().contains("windowsapps") {
            continue;
        }
        let cand = dir.join(&exe_name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 判断两个目录是否指向同一位置（优先 canonicalize，失败回退大小写无关字符串比较）。
fn same_dir(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
        return ca == cb;
    }
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(|c| c == '\\' || c == '/')
            .to_lowercase()
    };
    norm(a) == norm(b)
}
