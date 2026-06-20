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
pub async fn list_remote(source: String, refresh: bool) -> Result<Vec<RemoteInfo>, String> {
    // 网络拉取放到阻塞线程池，避免阻塞 UI 线程（修复加载卡死）。
    tauri::async_runtime::spawn_blocking(move || list_remote_blocking(source, refresh))
        .await
        .map_err(|e| e.to_string())?
}

fn list_remote_blocking(source: String, refresh: bool) -> Result<Vec<RemoteInfo>, String> {
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
    proxy: String,
    use_uv: bool,
    uv_version: Option<String>,
}

#[tauri::command]
pub fn get_config() -> Result<ConfigInfo, String> {
    let p = paths()?;
    let c = Config::load(&p).map_err(|e| e.to_string())?;
    Ok(ConfigInfo {
        default_source: c.default_source_resolved().cli_value().to_string(),
        pip_mirror: c.pip_mirror,
        proxy: c.proxy.unwrap_or_else(|| "direct".into()),
        use_uv: c.use_uv.unwrap_or(false),
        uv_version: pvm::pkg::uv_version(),
    })
}

/// 设置网络代理模式（"direct"/"system"/自定义 URL），写入 config，重启 app 生效。
#[tauri::command]
pub fn set_proxy(mode: String) -> Result<(), String> {
    let p = paths()?;
    let mut c = Config::load(&p).map_err(|e| e.to_string())?;
    let mode = mode.trim();
    c.proxy = Some(if mode.is_empty() { "direct".into() } else { mode.to_string() });
    c.save(&p).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct AiConfigInfo {
    provider: String,
    base_url: String,
    model: String,
    has_key: bool,
}

#[tauri::command]
pub fn get_ai_config() -> Result<AiConfigInfo, String> {
    let p = paths()?;
    let c = Config::load(&p).map_err(|e| e.to_string())?;
    Ok(AiConfigInfo {
        provider: c.ai_provider.unwrap_or_else(|| "openai".into()),
        base_url: c.ai_base_url.unwrap_or_default(),
        model: c.ai_model.unwrap_or_default(),
        has_key: c.ai_key.map_or(false, |k| !k.trim().is_empty()),
    })
}

#[tauri::command]
pub fn set_ai_config(
    provider: String,
    base_url: String,
    key: String,
    model: String,
) -> Result<(), String> {
    let p = paths()?;
    let mut c = Config::load(&p).map_err(|e| e.to_string())?;
    c.ai_provider = Some(provider);
    c.ai_base_url = if base_url.trim().is_empty() { None } else { Some(base_url) };
    // 空 key 表示不修改（避免清空已存的 key）
    if !key.trim().is_empty() {
        c.ai_key = Some(key);
    }
    c.ai_model = if model.trim().is_empty() { None } else { Some(model) };
    c.save(&p).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ai_diagnose(error_log: String) -> Result<String, String> {
    let p = paths()?;
    tauri::async_runtime::spawn_blocking(move || {
        let c = Config::load(&p).map_err(|e| e.to_string())?;
        let cfg = pvm::ai::AiConfig {
            provider: c.ai_provider.unwrap_or_else(|| "openai".into()),
            base_url: c
                .ai_base_url
                .filter(|s| !s.trim().is_empty())
                .ok_or("未配置 AI Base URL（在设置页填写）")?,
            key: c
                .ai_key
                .filter(|k| !k.trim().is_empty())
                .ok_or("未配置 AI API Key（在设置页填写）")?,
            model: c
                .ai_model
                .filter(|s| !s.trim().is_empty())
                .ok_or("未配置 AI 模型（在设置页填写）")?,
        };
        pvm::ai::diagnose(&cfg, &error_log).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// AI 自然语言找包：返回推荐的包（name + 推荐理由放在 summary）。
#[tauri::command]
pub async fn ai_find_packages(query: String) -> Result<Vec<pvm::pkg::SearchHit>, String> {
    let p = paths()?;
    tauri::async_runtime::spawn_blocking(move || {
        let c = Config::load(&p).map_err(|e| e.to_string())?;
        let cfg = pvm::ai::AiConfig {
            provider: c.ai_provider.unwrap_or_else(|| "openai".into()),
            base_url: c
                .ai_base_url
                .filter(|s| !s.trim().is_empty())
                .ok_or("未配置 AI Base URL（在设置页填写）")?,
            key: c
                .ai_key
                .filter(|k| !k.trim().is_empty())
                .ok_or("未配置 AI API Key（在设置页填写）")?,
            model: c
                .ai_model
                .filter(|s| !s.trim().is_empty())
                .ok_or("未配置 AI 模型（在设置页填写）")?,
        };
        let recs = pvm::ai::find_packages(&cfg, &query).map_err(|e| e.to_string())?;
        Ok(recs
            .into_iter()
            .map(|(name, reason)| pvm::pkg::SearchHit {
                name,
                version: String::new(),
                summary: reason,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- 批 3：漏洞扫描 / PATH 诊断 / 健康评分 / 依赖图 / uv ----------

#[tauri::command]
pub async fn osv_scan(py_exe: String) -> Result<Vec<pvm::osv::VulnHit>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let pkgs = pvm::pkg::list_packages(std::path::Path::new(&py_exe)).map_err(|e| e.to_string())?;
        let pairs: Vec<(String, String)> = pkgs.into_iter().map(|p| (p.name, p.version)).collect();
        pvm::osv::scan(&pairs).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn path_diag() -> Vec<pvm::system::PathPython> {
    pvm::system::path_pythons()
}

#[tauri::command]
pub async fn pkg_health(py_exe: String) -> Result<pvm::pkg::Health, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::health(std::path::Path::new(&py_exe)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_dep_graph(py_exe: String) -> Result<Vec<pvm::pkg::DepNode>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok::<_, String>(pvm::pkg::dep_graph(std::path::Path::new(&py_exe)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn uv_status() -> Option<String> {
    pvm::pkg::uv_version()
}

#[tauri::command]
pub fn set_use_uv(enabled: bool) -> Result<(), String> {
    let p = paths()?;
    let mut c = Config::load(&p).map_err(|e| e.to_string())?;
    c.use_uv = Some(enabled);
    c.save(&p).map_err(|e| e.to_string())
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
    proxy: String,
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
        proxy: pvm::net::detect_proxy().unwrap_or_else(|| "(直连)".into()),
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
pub async fn list_system_pythons() -> Vec<pvm::system::SystemPython> {
    tauri::async_runtime::spawn_blocking(pvm::system::list_system_pythons)
        .await
        .unwrap_or_default()
}

// ---------- 解释器枚举 + 包管理 ----------

#[derive(Serialize)]
pub struct Interpreter {
    label: String,
    py_exe: String,
    kind: String,
}

/// 汇总所有可管理的解释器：pvm 安装的 + 虚拟环境 + 系统已装。
#[tauri::command]
pub async fn list_interpreters() -> Result<Vec<Interpreter>, String> {
    tauri::async_runtime::spawn_blocking(list_interpreters_blocking)
        .await
        .map_err(|e| e.to_string())?
}

fn list_interpreters_blocking() -> Result<Vec<Interpreter>, String> {
    let p = paths()?;
    let mut out = Vec::new();
    for v in resolve::list_installed(&p).unwrap_or_default() {
        out.push(Interpreter {
            label: format!("pvm · {} ({})", v.xyz(), v.source.id_suffix()),
            py_exe: p.python_exe(&v).display().to_string(),
            kind: "pvm".into(),
        });
    }
    for m in venv::venv_list(&p).unwrap_or_default() {
        let py = p.venvs().join(&m.name).join("Scripts").join("python.exe");
        if py.exists() {
            out.push(Interpreter {
                label: format!("venv · {}", m.name),
                py_exe: py.display().to_string(),
                kind: "venv".into(),
            });
        }
    }
    for s in pvm::system::list_system_pythons() {
        out.push(Interpreter {
            label: format!("系统 · Python {}", s.version),
            py_exe: s.path,
            kind: "system".into(),
        });
    }
    Ok(out)
}

fn resolve_mirror_url(alias: Option<String>) -> Option<String> {
    alias.and_then(|a| pvm::pip::lookup_mirror(&a).map(|m| m.index_url.to_string()))
}

#[tauri::command]
pub async fn pkg_list(py_exe: String) -> Result<Vec<pvm::pkg::Package>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::list_packages(std::path::Path::new(&py_exe)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_outdated(py_exe: String) -> Result<Vec<pvm::pkg::Outdated>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::list_outdated(std::path::Path::new(&py_exe)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_install(
    py_exe: String,
    spec: String,
    mirror: Option<String>,
    upgrade: bool,
) -> Result<String, String> {
    let p = paths()?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = resolve_mirror_url(mirror);
        let use_uv = Config::load(&p).ok().and_then(|c| c.use_uv).unwrap_or(false);
        pvm::pkg::install(std::path::Path::new(&py_exe), &spec, url.as_deref(), upgrade, use_uv)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_uninstall(py_exe: String, name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::uninstall(std::path::Path::new(&py_exe), &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_freeze(py_exe: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::freeze(std::path::Path::new(&py_exe)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_install_requirements(
    py_exe: String,
    req_file: String,
    mirror: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = resolve_mirror_url(mirror);
        pvm::pkg::install_requirements(
            std::path::Path::new(&py_exe),
            std::path::Path::new(&req_file),
            url.as_deref(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 本地详情：纯 Rust 读 METADATA，秒出（不启动 python、不联网）。
#[tauri::command]
pub async fn pkg_detail(py_exe: String, name: String) -> Result<pvm::pkg::PkgDetail, String> {
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::local_detail(std::path::Path::new(&py_exe), &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// PyPI 元数据：可用版本 / README / 链接，详情面板异步补充（网络）。
#[tauri::command]
pub async fn pkg_pypi(name: String) -> Result<pvm::pkg::PypiInfo, String> {
    tauri::async_runtime::spawn_blocking(move || pvm::pkg::pypi_info(&name).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// 打开一个新 PowerShell 窗口，把选定解释器目录前置到本会话 PATH（python/pip 即指向它）。
#[tauri::command]
pub fn open_terminal(py_exe: String) -> Result<(), String> {
    let py = PathBuf::from(&py_exe);
    let dir = py.parent().ok_or_else(|| "无效解释器路径".to_string())?;
    let scripts = dir.join("Scripts");
    let cmd = format!(
        "$env:Path='{};{};'+$env:Path; Write-Host 'pvm: 本会话 python 已指向 {}' -ForegroundColor Green; python --version",
        dir.display(),
        scripts.display(),
        dir.display()
    );
    let mut c = std::process::Command::new("powershell");
    c.args(["-NoExit", "-Command", &cmd]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
    }
    c.spawn().map_err(|e| format!("启动终端失败: {e}"))?;
    Ok(())
}

/// 用系统默认浏览器打开 http(s) 链接（仅允许 http/https）。
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("仅允许 http(s) 链接".into());
    }
    let mut c = std::process::Command::new("rundll32.exe");
    c.args(["url.dll,FileProtocolHandler", &url]);
    c.spawn().map_err(|e| format!("打开链接失败: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn pkg_search(query: String) -> Result<Vec<pvm::pkg::SearchHit>, String> {
    let p = paths()?;
    tauri::async_runtime::spawn_blocking(move || {
        pvm::pkg::search_pypi(&query, &p.cache()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pkg_dry_run(
    py_exe: String,
    spec: String,
    mirror: Option<String>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = resolve_mirror_url(mirror);
        pvm::pkg::dry_run(std::path::Path::new(&py_exe), &spec, url.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 批量操作（install/upgrade/uninstall），后台线程顺序执行，pip 输出流式 emit 到前端。
#[tauri::command]
pub fn pkg_batch(
    app: AppHandle,
    py_exe: String,
    action: String,
    names: Vec<String>,
    mirror: Option<String>,
) -> Result<(), String> {
    let url = resolve_mirror_url(mirror);
    let use_uv = paths()
        .ok()
        .and_then(|p| Config::load(&p).ok())
        .and_then(|c| c.use_uv)
        .unwrap_or(false);
    std::thread::spawn(move || {
        let py = PathBuf::from(&py_exe);
        let app_line = app.clone();
        let on_line = move |line: &str| {
            let _ = app_line.emit("batch://line", line.to_string());
        };
        let total = names.len();
        for (i, name) in names.iter().enumerate() {
            let _ = app.emit(
                "batch://item",
                serde_json::json!({ "index": i + 1, "total": total, "name": name, "action": action }),
            );
            let r = match action.as_str() {
                "uninstall" => pvm::pkg::uninstall_stream(&py, name, &on_line),
                "upgrade" => pvm::pkg::install_stream(&py, name, url.as_deref(), true, use_uv, &on_line),
                _ => pvm::pkg::install_stream(&py, name, url.as_deref(), false, use_uv, &on_line),
            };
            let ok = r.unwrap_or(false);
            let _ = app.emit(
                "batch://item-done",
                serde_json::json!({ "name": name, "success": ok }),
            );
        }
        let _ = app.emit("batch://done", serde_json::json!({ "total": total }));
    });
    Ok(())
}

// ---------- 环境快照 / 克隆 / 项目脚手架 ----------

#[tauri::command]
pub async fn snapshot_save(py_exe: String, name: String, py_label: String) -> Result<(), String> {
    let p = paths()?;
    tauri::async_runtime::spawn_blocking(move || {
        pvm::snapshot::save(
            std::path::Path::new(&py_exe),
            &name,
            &py_label,
            &p.root.join("snapshots"),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn snapshot_list() -> Result<Vec<pvm::snapshot::Snapshot>, String> {
    let p = paths()?;
    Ok(pvm::snapshot::list(&p.root.join("snapshots")))
}

#[tauri::command]
pub fn snapshot_delete(name: String) -> Result<(), String> {
    let p = paths()?;
    pvm::snapshot::delete(&p.root.join("snapshots"), &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn snapshot_apply(
    py_exe: String,
    name: String,
    mirror: Option<String>,
) -> Result<String, String> {
    let p = paths()?;
    tauri::async_runtime::spawn_blocking(move || {
        let snap =
            pvm::snapshot::load(&p.root.join("snapshots"), &name).map_err(|e| e.to_string())?;
        let url = resolve_mirror_url(mirror);
        pvm::snapshot::apply(std::path::Path::new(&py_exe), &snap, url.as_deref(), &p.cache())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 项目脚手架：在目录写 .python-version + 建 .venv（基于指定版本）。
#[tauri::command]
pub fn scaffold(dir: String, canonical: String, mirror: Option<String>) -> Result<String, String> {
    let p = paths()?;
    let target = PathBuf::from(&dir);
    if !target.is_dir() {
        return Err(format!("目录不存在: {dir}"));
    }
    let v = PythonVersion::parse_canonical(&canonical).map_err(|e| e.to_string())?;
    if !resolve::is_installed(&v, &p) {
        return Err(format!("未安装: {canonical}"));
    }
    std::fs::write(target.join(".python-version"), v.canonical()).map_err(|e| e.to_string())?;
    let venv_path = target.join(".venv");
    let opts = VenvCreateOpts {
        name: "project",
        py_selector: Some(&canonical),
        in_project: false,
        path: Some(&venv_path),
        clear: false,
        without_pip: false,
        system_site_packages: false,
        mirror: mirror.as_deref(),
    };
    let created = venv::venv_create(&opts, &p).map_err(|e| e.to_string())?;
    Ok(format!(
        "已创建项目环境：\n.python-version → {}\n.venv → {}",
        v.canonical(),
        created.display()
    ))
}
