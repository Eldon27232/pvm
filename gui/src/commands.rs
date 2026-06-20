//! pvm GUI 后端命令：桥接 pvm core，向前端暴露 Tauri command。
//! 长任务（安装）在后台线程执行，进度/结果通过 Tauri 事件推送前端。

use pvm::config::Config;
use pvm::paths::Paths;
use pvm::pip::{self, Scope, MIRRORS};
use pvm::resolve::{self, ResolvedFrom};
use pvm::venv::{self, VenvCreateOpts};
use pvm::version::{parse_selector, PythonVersion, Source};
use pvm::{source_pbs, source_pyorg};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

// ---------- 公共辅助 ----------

fn paths() -> Result<Paths, String> {
    Paths::discover(None).map_err(|e| e.to_string())
}

fn parse_source(s: &str) -> Result<Source, String> {
    Source::from_cli(s).ok_or_else(|| format!("未知来源: {s}"))
}

/// 解析全局默认版本（global 文件可能存 selector，需 resolve 到具体版本）。
fn global_version(p: &Paths) -> Option<PythonVersion> {
    let raw = std::fs::read_to_string(p.global_version_file()).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let sel = parse_selector(raw).ok()?;
    let cfg = Config::load(p).ok()?;
    resolve::resolve_installed(&sel, cfg.default_source_resolved(), p).ok()
}

// ---------- 基础信息 ----------

#[tauri::command]
pub fn root_dir() -> Result<String, String> {
    Ok(paths()?.root.display().to_string())
}

#[derive(Serialize)]
pub struct InstalledInfo {
    id: String,
    version: String,
    source: String,
    freethreaded: bool,
    path: String,
    is_global: bool,
}

#[tauri::command]
pub fn list_installed() -> Result<Vec<InstalledInfo>, String> {
    let p = paths()?;
    let gv = global_version(&p);
    let list = resolve::list_installed(&p).map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|v| InstalledInfo {
            is_global: gv.as_ref() == Some(&v),
            id: v.canonical(),
            version: v.xyz(),
            source: v.source.id_suffix().to_string(),
            freethreaded: v.freethreaded,
            path: p.python_exe(&v).display().to_string(),
        })
        .collect())
}

#[derive(Serialize)]
pub struct CurrentInfo {
    id: String,
    version: String,
    source: String,
    from: String,
    dir: String,
}

#[tauri::command]
pub fn current_version() -> Result<Option<CurrentInfo>, String> {
    let p = paths()?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    match resolve::resolve_effective(&cwd, &p) {
        Ok(eff) => Ok(Some(CurrentInfo {
            id: eff.version.canonical(),
            version: eff.version.xyz(),
            source: eff.version.source.id_suffix().to_string(),
            from: match eff.from {
                ResolvedFrom::Shell => "shell",
                ResolvedFrom::Local(_) => "local",
                ResolvedFrom::Global => "global",
            }
            .to_string(),
            dir: eff.interpreter_dir.display().to_string(),
        })),
        Err(_) => Ok(None),
    }
}

// ---------- 远程版本 ----------

#[derive(Serialize)]
pub struct RemoteInfo {
    version: String,
    source: String,
    freethreaded: bool,
    installed: bool,
    date: Option<String>,
    size: Option<u64>,
}

#[tauri::command]
pub fn list_remote(source: String, refresh: bool) -> Result<Vec<RemoteInfo>, String> {
    let p = paths()?;
    let src = parse_source(&source)?;
    let installed: std::collections::HashSet<String> = resolve::list_installed(&p)
        .map_err(|e| e.to_string())?
        .iter()
        .map(|v| v.canonical())
        .collect();
    let token = std::env::var("GITHUB_TOKEN").ok();
    match src {
        Source::Standalone => {
            let assets = source_pbs::list_pbs_assets(
                token.as_deref(),
                source_pbs::PbsFlavor::InstallOnly,
                false,
                &p,
                refresh,
            )
            .map_err(|e| e.to_string())?;
            Ok(assets
                .into_iter()
                .map(|a| {
                    let ver = a.python_version.to_string();
                    let id = format!("cpython-{ver}-standalone");
                    RemoteInfo {
                        installed: installed.contains(&id),
                        version: ver,
                        source: "standalone".into(),
                        freethreaded: a.freethreaded,
                        date: Some(a.release_date),
                        size: Some(a.size),
                    }
                })
                .collect())
        }
        Source::Org => {
            let rels = source_pyorg::list_remote(&p, refresh).map_err(|e| e.to_string())?;
            Ok(rels
                .into_iter()
                .map(|r| {
                    let ver = r.version.to_string();
                    let id = format!("cpython-{ver}-org");
                    RemoteInfo {
                        installed: installed.contains(&id),
                        version: ver,
                        source: "org".into(),
                        freethreaded: false,
                        date: None,
                        size: None,
                    }
                })
                .collect())
        }
    }
}

// ---------- 安装（后台线程 + 进度事件）----------

fn emit_done(app: &AppHandle, id: &str, success: bool, error: &str) {
    let _ = app.emit(
        "install://done",
        serde_json::json!({ "id": id, "success": success, "error": error }),
    );
}

fn find_pbs_asset(p: &Paths, v: &PythonVersion) -> Result<source_pbs::PbsAsset, String> {
    let token = std::env::var("GITHUB_TOKEN").ok();
    let assets = source_pbs::list_pbs_assets(
        token.as_deref(),
        source_pbs::PbsFlavor::InstallOnly,
        v.freethreaded,
        p,
        false,
    )
    .map_err(|e| e.to_string())?;
    assets
        .into_iter()
        .find(|a| {
            a.python_version.major == v.major as u64
                && a.python_version.minor == v.minor as u64
                && a.python_version.patch == v.patch as u64
        })
        .ok_or_else(|| format!("standalone 无此版本资产: {}", v.xyz()))
}

#[tauri::command]
pub fn install(
    app: AppHandle,
    version: String,
    source: String,
    freethreaded: bool,
    threads: usize,
    set_global: bool,
) -> Result<(), String> {
    let src = parse_source(&source)?;
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("版本号需为 x.y.z：{version}"));
    }
    let major = parts[0].parse::<u32>().map_err(|_| "非法主版本号".to_string())?;
    let minor = parts[1].parse::<u32>().map_err(|_| "非法次版本号".to_string())?;
    let patch = parts[2].parse::<u32>().map_err(|_| "非法补丁号".to_string())?;
    let v = PythonVersion {
        source: src,
        major,
        minor,
        patch,
        freethreaded,
    };
    let id = v.canonical();
    let n = if threads == 0 { 8 } else { threads.min(16) };

    std::thread::spawn(move || {
        let p = match Paths::discover(None) {
            Ok(p) => p,
            Err(e) => {
                emit_done(&app, &id, false, &e.to_string());
                return;
            }
        };
        let _ = app.emit("install://start", serde_json::json!({ "id": id }));

        let app_cb = app.clone();
        let id_cb = id.clone();
        let on_progress = move |done: u64, total: u64| {
            let _ = app_cb.emit(
                "install://progress",
                serde_json::json!({ "id": id_cb, "downloaded": done, "total": total }),
            );
        };

        let result = match src {
            Source::Standalone => match find_pbs_asset(&p, &v) {
                Ok(asset) => source_pbs::install_pbs_progress(&asset, &v, &p, false, n, &on_progress)
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e),
            },
            Source::Org => {
                let _ = app.emit(
                    "install://stage",
                    serde_json::json!({ "id": id, "stage": "download" }),
                );
                let r = source_pyorg::install_via_installer_progress(&v, &p, n, &on_progress)
                    .map_err(|e| e.to_string());
                let _ = app.emit(
                    "install://stage",
                    serde_json::json!({ "id": id, "stage": "install" }),
                );
                r
            }
        };

        match result {
            Ok(()) => {
                if set_global {
                    let _ = std::fs::write(p.global_version_file(), v.canonical());
                }
                emit_done(&app, &id, true, "");
            }
            Err(e) => emit_done(&app, &id, false, &e),
        }
    });
    Ok(())
}

#[tauri::command]
pub fn uninstall(id: String) -> Result<(), String> {
    let p = paths()?;
    let v = PythonVersion::parse_canonical(&id).map_err(|e| e.to_string())?;
    match v.source {
        Source::Standalone => {
            let dir = p.version_dir(&v);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
        }
        Source::Org => source_pyorg::uninstall_via_installer(&v, &p).map_err(|e| e.to_string())?,
    }
    Ok(())
}

// ---------- 版本切换 ----------

#[tauri::command]
pub fn set_global(id: String) -> Result<(), String> {
    let p = paths()?;
    let v = PythonVersion::parse_canonical(&id).map_err(|e| e.to_string())?;
    if !resolve::is_installed(&v, &p) {
        return Err(format!("未安装: {id}"));
    }
    std::fs::create_dir_all(&p.root).map_err(|e| e.to_string())?;
    std::fs::write(p.global_version_file(), v.canonical()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_local(id: String, dir: String) -> Result<(), String> {
    let v = PythonVersion::parse_canonical(&id).map_err(|e| e.to_string())?;
    let target = PathBuf::from(&dir);
    if !target.is_dir() {
        return Err(format!("目录不存在: {dir}"));
    }
    std::fs::write(target.join(".python-version"), v.canonical()).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 虚拟环境 ----------

#[derive(Serialize)]
pub struct VenvInfo {
    name: String,
    python_version: String,
    source: String,
    created_at: String,
    path: String,
}

#[tauri::command]
pub fn venv_list() -> Result<Vec<VenvInfo>, String> {
    let p = paths()?;
    let list = venv::venv_list(&p).map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|m| VenvInfo {
            path: p.venvs().join(&m.name).display().to_string(),
            name: m.name,
            python_version: m.python_version,
            source: m.source,
            created_at: m.created_at,
        })
        .collect())
}

#[tauri::command]
pub fn venv_create(name: String, selector: String, mirror: Option<String>) -> Result<String, String> {
    let p = paths()?;
    let opts = VenvCreateOpts {
        name: &name,
        py_selector: Some(&selector),
        in_project: false,
        path: None,
        clear: false,
        without_pip: false,
        system_site_packages: false,
        mirror: mirror.as_deref(),
    };
    let target = venv::venv_create(&opts, &p).map_err(|e| e.to_string())?;
    Ok(venv::activation_hint(&target))
}

#[tauri::command]
pub fn venv_remove(name: String) -> Result<(), String> {
    let p = paths()?;
    venv::venv_remove(&name, &p, true).map_err(|e| e.to_string())
}

// ---------- pip 镜像 ----------

#[derive(Serialize)]
pub struct MirrorInfo {
    alias: String,
    display: String,
    index_url: String,
}

#[tauri::command]
pub fn mirror_list() -> Vec<MirrorInfo> {
    MIRRORS
        .iter()
        .map(|m| MirrorInfo {
            alias: m.aliases[0].to_string(),
            display: m.display.to_string(),
            index_url: m.index_url.to_string(),
        })
        .collect()
}

#[derive(Serialize)]
pub struct MirrorCurrent {
    index_url: Option<String>,
    trusted_host: Option<String>,
}

#[tauri::command]
pub fn mirror_current() -> Result<MirrorCurrent, String> {
    let p = paths()?;
    match pip::pip_mirror_current(Scope::Global, None, &p).map_err(|e| e.to_string())? {
        Some((idx, th)) => Ok(MirrorCurrent {
            index_url: Some(idx),
            trusted_host: th,
        }),
        None => Ok(MirrorCurrent {
            index_url: None,
            trusted_host: None,
        }),
    }
}

#[tauri::command]
pub fn mirror_set(name_or_url: String) -> Result<(), String> {
    let p = paths()?;
    pip::pip_mirror_set(&name_or_url, Scope::Global, None, false, &p).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn mirror_reset() -> Result<(), String> {
    let p = paths()?;
    pip::pip_mirror_reset(Scope::Global, None, &p).map_err(|e| e.to_string())
}

// ---------- 配置 ----------

#[derive(Serialize)]
pub struct ConfigInfo {
    default_source: String,
    pip_mirror: Option<String>,
}

#[tauri::command]
pub fn get_config() -> Result<ConfigInfo, String> {
    let p = paths()?;
    let c = Config::load(&p).map_err(|e| e.to_string())?;
    Ok(ConfigInfo {
        default_source: c.default_source_resolved().cli_value().to_string(),
        pip_mirror: c.pip_mirror,
    })
}

#[tauri::command]
pub fn set_default_source(source: String) -> Result<(), String> {
    let p = paths()?;
    let _ = parse_source(&source)?;
    let mut c = Config::load(&p).map_err(|e| e.to_string())?;
    c.default_source = Some(source);
    c.save(&p).map_err(|e| e.to_string())
}

// ---------- 诊断 / 初始化 ----------

#[derive(Serialize)]
pub struct DoctorInfo {
    root: String,
    shim_ready: bool,
    shims_in_path: bool,
    global: Option<String>,
    installed_count: usize,
}

#[tauri::command]
pub fn doctor() -> Result<DoctorInfo, String> {
    let p = paths()?;
    let shims_str = p.shims().to_string_lossy().to_lowercase();
    let in_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .any(|x| x.trim().to_lowercase() == shims_str);
    Ok(DoctorInfo {
        shim_ready: p.bin().join("pvm-shim.exe").exists(),
        shims_in_path: in_path,
        global: global_version(&p).map(|v| v.canonical()),
        installed_count: resolve::list_installed(&p).map(|v| v.len()).unwrap_or(0),
        root: p.root.display().to_string(),
    })
}

#[tauri::command]
pub fn init_pvm() -> Result<String, String> {
    let p = paths()?;
    for d in [
        p.bin(),
        p.shims(),
        p.versions(),
        p.venvs(),
        p.cache(),
        p.logs(),
        p.backup(),
    ] {
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    }
    // 从 GUI exe 同目录拷 shim 模板（分发时三者同目录）
    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = cur.parent().map(|x| x.to_path_buf()).unwrap_or_default();
    let mut missing = Vec::new();
    for n in ["pvm-shim.exe", "pvm-shimw.exe", "pvm.exe"] {
        let src = dir.join(n);
        if src.exists() {
            let _ = std::fs::copy(&src, p.bin().join(n));
        } else {
            missing.push(n);
        }
    }
    pvm::shim::rehash(&p).map_err(|e| e.to_string())?;
    let shims = p.shims().to_string_lossy().to_string();
    pvm::winpath::prepend_shims_to_user_path(&shims, &p).map_err(|e| e.to_string())?;

    let mut msg = format!(
        "已初始化：{}\nshims 已加入用户 PATH（请重开终端生效）",
        p.root.display()
    );
    if !missing.is_empty() {
        msg.push_str(&format!(
            "\n注意：未找到 {}（命令行 shim 暂不可用，请确保这些 exe 与 GUI 同目录后重试 init）",
            missing.join(", ")
        ));
    }
    Ok(msg)
}

#[tauri::command]
pub fn list_system_pythons() -> Vec<pvm::system::SystemPython> {
    pvm::system::list_system_pythons()
}
