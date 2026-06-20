//! 命令分发与实现。

use crate::cli::{Cli, Command, FlavorArg, MirrorCmd, SourceArg, VenvCmd};
use crate::config::Config;
use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::pip::{self, Scope};
use crate::resolve::{
    find_dotfile_upwards, is_installed, list_installed, resolve_effective, resolve_installed,
    ResolvedFrom,
};
use crate::source_pbs::{self, PbsFlavor};
use crate::source_pyorg;
use crate::version::{parse_selector, PythonVersion, Source, VersionSelector};
use crate::{shim, venv, winpath};
use std::path::Path;
use std::process::Command as ProcCommand;

pub fn run(cli: Cli) -> Result<()> {
    let paths = Paths::discover(cli.root.clone())?;
    let config = Config::load(&paths)?;
    let default_source = config.default_source_resolved();
    let yes = cli.yes;

    match cli.command {
        Command::Install {
            versions,
            source,
            flavor,
            freethreaded,
            force,
            skip_existing,
            set_global,
            no_verify,
            mirror,
        } => cmd_install(
            versions, source, flavor, freethreaded, force, skip_existing, set_global, no_verify,
            mirror, default_source, &paths,
        )?,
        Command::Uninstall { versions, keep_venvs } => {
            for vs in &versions {
                let v = resolve_installed(&parse_selector(vs)?, default_source, &paths)?;
                println!("卸载 {} ...", v.canonical());
                match v.source {
                    Source::Standalone => {
                        let dir = paths.version_dir(&v);
                        if dir.exists() {
                            std::fs::remove_dir_all(&dir)?;
                        }
                    }
                    Source::Org => source_pyorg::uninstall_via_installer(&v, &paths)?,
                }
                println!("已卸载 {}", v.canonical());
            }
            let _ = keep_venvs;
            shim::rehash(&paths).ok();
        }
        Command::List {
            remote,
            source,
            bare,
            all,
        } => {
            if remote {
                list_remote_cmd(source, all, false, &paths)?;
            } else {
                let installed = list_installed(&paths)?;
                if installed.is_empty() {
                    println!("（未安装任何版本，用 pvm install <版本>）");
                }
                let g = read_global(&paths);
                for v in &installed {
                    if bare {
                        println!("{}", v.canonical());
                    } else {
                        let mark = if Some(v.canonical()) == g { "*" } else { " " };
                        println!("{mark} {}", v.canonical());
                    }
                }
            }
        }
        Command::LsRemote {
            source,
            all,
            refresh,
        } => list_remote_cmd(source, all, refresh, &paths)?,
        Command::Global { version } => match version {
            None => println!("{}", read_global(&paths).unwrap_or_else(|| "(未设置)".into())),
            Some(vs) => {
                let v = resolve_installed(&parse_selector(&vs)?, default_source, &paths)?;
                write_global(&paths, &v)?;
                shim::rehash(&paths).ok();
                println!("全局版本设为 {}", v.canonical());
            }
        },
        Command::Local { version, unset } => {
            let cwd = std::env::current_dir()?;
            let dotfile = cwd.join(".python-version");
            if unset {
                if dotfile.exists() {
                    std::fs::remove_file(&dotfile)?;
                    println!("已移除 {}", dotfile.display());
                } else {
                    println!("当前目录无 .python-version");
                }
            } else {
                match version {
                    None => match find_dotfile_upwards(&cwd)? {
                        Some((f, raw)) => println!("{}  （来自 {}）", raw.trim(), f.display()),
                        None => println!("(无 .python-version)"),
                    },
                    Some(vs) => {
                        let v = resolve_installed(&parse_selector(&vs)?, default_source, &paths)?;
                        std::fs::write(&dotfile, v.canonical())?;
                        shim::rehash(&paths).ok();
                        println!("已写入 {} -> {}", dotfile.display(), v.canonical());
                    }
                }
            }
        }
        Command::Shell { version, unset } => {
            if unset {
                println!("请运行以清除会话版本：");
                println!("  PowerShell : $env:PVM_VERSION=$null");
                println!("  cmd.exe    : set PVM_VERSION=");
            } else {
                match version {
                    None => println!(
                        "{}",
                        std::env::var("PVM_VERSION").unwrap_or_else(|_| "(未设置)".into())
                    ),
                    Some(vs) => {
                        let v = resolve_installed(&parse_selector(&vs)?, default_source, &paths)?;
                        println!("请运行以在当前会话生效：");
                        println!("  PowerShell : $env:PVM_VERSION='{}'", v.canonical());
                        println!("  cmd.exe    : set PVM_VERSION={}", v.canonical());
                    }
                }
            }
        }
        Command::Which { exe } => {
            let eff = resolve_effective(&std::env::current_dir()?, &paths)?;
            let stem = exe.unwrap_or_else(|| "python".into());
            let target = match stem.as_str() {
                "python" => paths.python_exe(&eff.version),
                "pythonw" => paths.pythonw_exe(&eff.version),
                other => paths.scripts_dir(&eff.version).join(format!("{other}.exe")),
            };
            println!("{}", target.display());
        }
        Command::Current => {
            let eff = resolve_effective(&std::env::current_dir()?, &paths)?;
            let from = match eff.from {
                ResolvedFrom::Shell => "PVM_VERSION".to_string(),
                ResolvedFrom::Local(p) => format!(".python-version ({})", p.display()),
                ResolvedFrom::Global => "global".to_string(),
            };
            println!("{}  （来源: {from}）", eff.version.canonical());
        }
        Command::Exec { version, cmd } => {
            if cmd.is_empty() {
                return Err(PvmError::Config(
                    "exec 需要命令，例：pvm exec -- python -V".into(),
                ));
            }
            let v = match version {
                Some(vs) => resolve_installed(&parse_selector(&vs)?, default_source, &paths)?,
                None => resolve_effective(&std::env::current_dir()?, &paths)?.version,
            };
            let py_exe = paths.python_exe(&v);
            let py_dir = py_exe.parent().unwrap_or(Path::new("."));
            let scripts = paths.scripts_dir(&v);
            let old = std::env::var("PATH").unwrap_or_default();
            let newpath = format!("{};{};{}", py_dir.display(), scripts.display(), old);
            let status = ProcCommand::new(&cmd[0])
                .args(&cmd[1..])
                .env("PATH", newpath)
                .status()
                .map_err(|e| PvmError::Config(format!("运行 {} 失败: {e}", cmd[0])))?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Venv { cmd } => cmd_venv(cmd, yes, &paths)?,
        Command::PipMirror { cmd } => cmd_mirror(cmd, &paths)?,
        Command::Init { path_only } => cmd_init(&paths, path_only)?,
        Command::Rehash => {
            shim::rehash(&paths)?;
            println!("rehash 完成");
        }
        Command::Doctor => cmd_doctor(&paths)?,
        Command::Root => println!("{}", paths.root.display()),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_install(
    versions: Vec<String>,
    source: Option<SourceArg>,
    flavor: FlavorArg,
    freethreaded: bool,
    force: bool,
    skip_existing: bool,
    set_global: bool,
    no_verify: bool,
    mirror: Option<String>,
    default_source: Source,
    paths: &Paths,
) -> Result<()> {
    if versions.is_empty() {
        return Err(PvmError::Config("请指定要安装的版本，例：pvm install 3.12".into()));
    }
    let flv = flavor.to_flavor();
    if matches!(flv, PbsFlavor::PgoFull) && !cfg!(feature = "zstd-full") {
        return Err(PvmError::Usage(
            "本二进制未启用 zstd-full，无法安装 --flavor full（请改用默认 install_only）".into(),
        ));
    }
    let cli_source = source.map(|s| s.to_source());
    let mut last: Option<PythonVersion> = None;
    for vs in &versions {
        let sel = parse_selector(vs)?;
        if sel.is_system() {
            return Err(PvmError::Usage(
                "不支持安装 system（system 指 PATH 上 pvm 之外的 Python）".into(),
            ));
        }
        // 合并来源：selector 的 @source 与 --source；都给且不同则报错
        let src = match (sel.source(), cli_source) {
            (Some(a), Some(b)) if a != b => {
                return Err(PvmError::Usage(format!(
                    "选择符来源 @{} 与 --source {} 冲突",
                    a.id_suffix(),
                    b.cli_value()
                )));
            }
            (Some(a), _) => a,
            (None, Some(b)) => b,
            (None, None) => default_source,
        };
        // 合并 free-threaded 意图：--freethreaded 或选择符的 t 后缀
        let eff_ft = freethreaded || sel.freethreaded();
        if src == Source::Org && (eff_ft || !matches!(flv, PbsFlavor::InstallOnly)) {
            return Err(PvmError::Usage(
                "free-threaded / 非 install_only flavor 仅 standalone 来源支持".into(),
            ));
        }
        let v = install_one(&sel, vs, src, flv, eff_ft, force, skip_existing, no_verify, paths)?;
        last = Some(v);
    }
    if set_global {
        if let Some(v) = &last {
            write_global(paths, v)?;
            println!("全局版本设为 {}", v.canonical());
        }
    }
    if let Some(m) = &mirror {
        pip::pip_mirror_set(m, Scope::Global, None, false, paths)?;
        println!("已设置全局 pip 镜像: {m}");
    }
    shim::rehash(paths).ok();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn install_one(
    sel: &VersionSelector,
    sel_str: &str,
    src: Source,
    flavor: PbsFlavor,
    freethreaded: bool,
    force: bool,
    skip_existing: bool,
    no_verify: bool,
    paths: &Paths,
) -> Result<PythonVersion> {
    match src {
        Source::Standalone => {
            let token = std::env::var("GITHUB_TOKEN").ok();
            let assets =
                source_pbs::list_pbs_assets(token.as_deref(), flavor, freethreaded, paths, false)?;
            let asset = assets
                .iter()
                .find(|a| sel_matches_ver(sel, &a.python_version))
                .ok_or_else(|| PvmError::VersionNotFound(sel_str.to_string()))?;
            let v = PythonVersion {
                source: Source::Standalone,
                major: asset.python_version.major as u32,
                minor: asset.python_version.minor as u32,
                patch: asset.python_version.patch as u32,
                freethreaded,
            };
            if is_installed(&v, paths) {
                if skip_existing {
                    println!("跳过已安装: {}", v.canonical());
                    return Ok(v);
                }
                if !force {
                    return Err(PvmError::Config(format!(
                        "{} 已安装（--force 重装 / --skip-existing 跳过）",
                        v.canonical()
                    )));
                }
            }
            println!("安装 {} ...", v.canonical());
            source_pbs::install_pbs(asset, &v, paths, no_verify)?;
            println!("已安装 {}", v.canonical());
            Ok(v)
        }
        Source::Org => {
            let rels = source_pyorg::list_remote(paths, false)?;
            let rel = rels
                .iter()
                .find(|r| sel_matches_ver(sel, &r.version))
                .ok_or_else(|| PvmError::VersionNotFound(sel_str.to_string()))?;
            let v = PythonVersion {
                source: Source::Org,
                major: rel.version.major as u32,
                minor: rel.version.minor as u32,
                patch: rel.version.patch as u32,
                freethreaded: false,
            };
            if is_installed(&v, paths) {
                if skip_existing {
                    println!("跳过已安装: {}", v.canonical());
                    return Ok(v);
                }
                if !force {
                    return Err(PvmError::Config(format!("{} 已安装", v.canonical())));
                }
            }
            println!("安装 {} (python.org) ...", v.canonical());
            source_pyorg::install_via_installer(&v, paths)?;
            println!("已安装 {}", v.canonical());
            Ok(v)
        }
    }
}

fn sel_matches_ver(sel: &VersionSelector, ver: &semver::Version) -> bool {
    match sel {
        VersionSelector::Exact { ver: (a, b, c), .. } => {
            ver.major == *a as u64 && ver.minor == *b as u64 && ver.patch == *c as u64
        }
        VersionSelector::PartialMinor { major, minor, .. } => {
            ver.major == *major as u64 && ver.minor == *minor as u64
        }
        VersionSelector::PartialMajor { major, .. } => ver.major == *major as u64,
        VersionSelector::Latest { .. } => true,
        VersionSelector::Canonical(v) => {
            ver.major == v.major as u64 && ver.minor == v.minor as u64 && ver.patch == v.patch as u64
        }
        VersionSelector::System => false,
    }
}

fn list_remote_cmd(
    source: Option<SourceArg>,
    _all: bool,
    refresh: bool,
    paths: &Paths,
) -> Result<()> {
    match source.map(|s| s.to_source()) {
        Some(Source::Org) => {
            for r in &source_pyorg::list_remote(paths, refresh)? {
                println!("{}", r.version);
            }
        }
        _ => {
            let token = std::env::var("GITHUB_TOKEN").ok();
            let assets = source_pbs::list_pbs_assets(
                token.as_deref(),
                PbsFlavor::InstallOnly,
                false,
                paths,
                refresh,
            )?;
            for a in &assets {
                println!("{}", a.python_version);
            }
        }
    }
    Ok(())
}

fn read_global(paths: &Paths) -> Option<String> {
    std::fs::read_to_string(paths.global_version_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_global(paths: &Paths, v: &PythonVersion) -> Result<()> {
    let gf = paths.global_version_file();
    if let Some(p) = gf.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = gf.with_extension("tmp");
    std::fs::write(&tmp, v.canonical())?;
    std::fs::rename(&tmp, &gf)?;
    Ok(())
}

fn cmd_venv(cmd: VenvCmd, yes: bool, paths: &Paths) -> Result<()> {
    match cmd {
        VenvCmd::Create {
            name,
            python,
            in_project,
            path,
            clear,
            without_pip,
            system_site_packages,
            mirror,
        } => {
            let opts = venv::VenvCreateOpts {
                name: &name,
                py_selector: python.as_deref(),
                in_project,
                path: path.as_deref(),
                clear,
                without_pip,
                system_site_packages,
                mirror: mirror.as_deref(),
            };
            let target = venv::venv_create(&opts, paths)?;
            println!("已创建 venv: {}", target.display());
            println!("{}", venv::activation_hint(&target));
            shim::rehash(paths).ok();
        }
        VenvCmd::List => {
            let list = venv::venv_list(paths)?;
            if list.is_empty() {
                println!("（无集中式 venv）");
            }
            for vm in &list {
                println!("{:20} {}", vm.name, vm.python_version);
            }
        }
        VenvCmd::Remove { name } => {
            venv::venv_remove(&name, paths, yes)?;
            println!("已删除 venv {name}");
            shim::rehash(paths).ok();
        }
        VenvCmd::Path { name } => println!("{}", venv::venv_path(&name, paths).display()),
        VenvCmd::Which { name } => println!(
            "{}",
            venv::venv_path(&name, paths)
                .join("Scripts")
                .join("python.exe")
                .display()
        ),
        VenvCmd::Activate { name } => {
            let s = venv::venv_path(&name, paths).join("Scripts");
            println!("& '{}\\Activate.ps1'", s.display());
        }
    }
    Ok(())
}

fn cmd_mirror(cmd: MirrorCmd, paths: &Paths) -> Result<()> {
    match cmd {
        MirrorCmd::Set {
            name_or_url,
            scope,
            no_trusted_host,
        } => {
            pip::pip_mirror_set(&name_or_url, scope.to_scope(), None, no_trusted_host, paths)?;
            println!("已设置 pip 镜像: {name_or_url}");
        }
        MirrorCmd::Show { scope } => pip::pip_mirror_show(scope.to_scope(), None, paths)?,
        MirrorCmd::List => pip::pip_mirror_list(),
        MirrorCmd::Reset { scope } => {
            pip::pip_mirror_reset(scope.to_scope(), None, paths)?;
            println!("已重置 pip 镜像");
        }
    }
    Ok(())
}

fn cmd_init(paths: &Paths, path_only: bool) -> Result<()> {
    for d in [
        paths.bin(),
        paths.shims(),
        paths.versions(),
        paths.venvs(),
        paths.cache(),
        paths.logs(),
        paths.backup(),
    ] {
        std::fs::create_dir_all(d)?;
    }

    let cur = std::env::current_exe()?;
    let dir = cur.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for n in ["pvm-shim.exe", "pvm-shimw.exe", "pvm.exe"] {
        let src = dir.join(n);
        if src.exists() {
            std::fs::copy(&src, paths.bin().join(n)).ok();
        } else if n != "pvm.exe" {
            eprintln!(
                "警告: 未找到 {}（shim 将不可用，请确保与 pvm.exe 同目录）",
                src.display()
            );
        }
    }

    if !path_only {
        shim::rehash(paths).ok();
    }

    let shims = paths.shims();
    let _shims_str = shims.to_string_lossy().to_string();
    #[cfg(windows)]
    winpath::prepend_shims_to_user_path(&_shims_str, paths)?;

    println!("已初始化 pvm。");
    println!("  shims: {}", shims.display());
    println!("请重开终端以使 PATH 生效。");
    Ok(())
}

fn cmd_doctor(paths: &Paths) -> Result<()> {
    println!("pvm 诊断：");
    println!("  root   = {}", paths.root.display());
    println!("  bin    = {}", paths.bin().display());
    println!("  shims  = {}", paths.shims().display());
    let shim_ok = paths.bin().join("pvm-shim.exe").exists();
    println!(
        "  shim 模板 = {}",
        if shim_ok { "已就位" } else { "缺失（运行 pvm init）" }
    );
    let installed = list_installed(paths).unwrap_or_default();
    println!("  已安装版本（{}）：", installed.len());
    for v in &installed {
        println!("    - {}", v.canonical());
    }
    println!(
        "  全局版本 = {}",
        read_global(paths).unwrap_or_else(|| "(未设置)".into())
    );
    let shims_str = paths.shims().to_string_lossy().to_lowercase();
    let in_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .any(|p| p.trim().to_lowercase() == shims_str);
    println!(
        "  shims 在 PATH = {}",
        if in_path { "是" } else { "否（运行 pvm init）" }
    );
    Ok(())
}
