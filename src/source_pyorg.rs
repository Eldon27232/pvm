//! python.org 来源：ftp 枚举、官方安装器静默安装/卸载、嵌入式（受限）安装。

use crate::archive::{extract, ArchiveKind};
use crate::download::{download_to, DownloadOpts};
use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::version::PythonVersion;
use std::io::Read;
use std::path::Path;
use std::process::Command;

/// 给子进程加 CREATE_NO_WINDOW，避免 release GUI（无 console）调用外部命令时闪现控制台窗口。
#[cfg(windows)]
fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
fn no_window(_c: &mut Command) {}

#[allow(dead_code)]
pub enum PyOrgFlavor {
    Installer,
    Embed,
}

#[derive(Debug, Clone)]
pub struct PyOrgRelease {
    pub version: semver::Version,
    pub is_prerelease: bool,
    pub installer_url: String,
    pub embed_url: String,
    pub sha256: Option<String>,
}

/// ftp 模板 URL 拼接：.../ftp/python/<X.Y.Z>/python-<X.Y.Z>-amd64.exe
pub fn installer_url(v: &PythonVersion) -> String {
    let xyz = v.xyz();
    format!("https://www.python.org/ftp/python/{xyz}/python-{xyz}-amd64.exe")
}

fn embed_url(xyz: &str) -> String {
    format!("https://www.python.org/ftp/python/{xyz}/python-{xyz}-embed-amd64.zip")
}

/// 枚举可安装版本：解析 ftp autoindex 目录列表（稳定回退路径）。
pub fn list_remote(paths: &Paths, refresh: bool) -> Result<Vec<PyOrgRelease>> {
    let cache = paths.cache().join("pyorg-index.json");
    if !refresh {
        if let Some(list) = load_cache(&cache) {
            return Ok(list);
        }
    }

    let url = "https://www.python.org/ftp/python/";
    let resp = crate::net::agent().get(url)
        .header("User-Agent", "pvm")
        .call()
        .map_err(|e| PvmError::Http(format!("拉取 python.org 版本列表失败: {e}")))?;
    let mut resp = resp;
    let mut html = String::new();
    resp.body_mut()
        .as_reader()
        .read_to_string(&mut html)
        .map_err(|e| PvmError::Http(e.to_string()))?;

    let re = regex::Regex::new(r#"href="(\d+\.\d+\.\d+)/""#).expect("版本目录正则应当合法");
    let mut seen = std::collections::BTreeSet::new();
    for cap in re.captures_iter(&html) {
        seen.insert(cap[1].to_string());
    }

    let mut out = Vec::new();
    for vstr in seen {
        if let Ok(ver) = semver::Version::parse(&vstr) {
            if ver.major != 3 {
                continue; // 仅 3.x 提供 amd64 安装器
            }
            let xyz = ver.to_string();
            out.push(PyOrgRelease {
                version: ver,
                is_prerelease: false,
                installer_url: format!(
                    "https://www.python.org/ftp/python/{xyz}/python-{xyz}-amd64.exe"
                ),
                embed_url: embed_url(&xyz),
                sha256: None,
            });
        }
    }
    out.sort_by(|a, b| b.version.cmp(&a.version));
    save_cache(&cache, &out);
    Ok(out)
}

fn load_cache(path: &Path) -> Option<Vec<PyOrgRelease>> {
    let text = std::fs::read_to_string(path).ok()?;
    let raw: Vec<(String, String, String, Option<String>)> = serde_json::from_str(&text).ok()?;
    let mut out = Vec::new();
    for (v, iu, eu, sha) in raw {
        if let Ok(ver) = semver::Version::parse(&v) {
            out.push(PyOrgRelease {
                version: ver,
                is_prerelease: false,
                installer_url: iu,
                embed_url: eu,
                sha256: sha,
            });
        }
    }
    Some(out)
}

fn save_cache(path: &Path, list: &[PyOrgRelease]) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let raw: Vec<(String, String, String, Option<String>)> = list
        .iter()
        .map(|r| {
            (
                r.version.to_string(),
                r.installer_url.clone(),
                r.embed_url.clone(),
                r.sha256.clone(),
            )
        })
        .collect();
    if let Ok(text) = serde_json::to_string(&raw) {
        let _ = std::fs::write(path, text);
    }
}

/// 官方安装器静默安装到版本目录（per-user，免 UAC；缓存 exe 以备卸载）。
pub fn install_via_installer(v: &PythonVersion, paths: &Paths) -> Result<()> {
    let exe = prepare_installer_exe(v, paths)?;
    download_to(&DownloadOpts {
        url: &installer_url(v),
        dest: &exe,
        expect_sha256: None,
        quiet: false,
    })?;
    install_from_installer_exe(v, paths, &exe)
}

/// 同 install_via_installer，但安装器用多线程下载并通过 on_progress 回调进度（GUI 用）。
pub fn install_via_installer_progress(
    v: &PythonVersion,
    paths: &Paths,
    threads: usize,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    let exe = prepare_installer_exe(v, paths)?;
    crate::download::download_parallel(&installer_url(v), &exe, None, threads, on_progress)?;
    install_from_installer_exe(v, paths, &exe)
}

fn prepare_installer_exe(v: &PythonVersion, paths: &Paths) -> Result<std::path::PathBuf> {
    let xyz = v.xyz();
    std::fs::create_dir_all(paths.cache())?;
    std::fs::create_dir_all(paths.versions())?;
    std::fs::create_dir_all(paths.logs())?;
    Ok(paths.cache().join(format!("python-{xyz}-amd64.exe")))
}

/// 已下载的安装器 exe → 静默安装 + 旧版本备份/回滚。
fn install_from_installer_exe(v: &PythonVersion, paths: &Paths, exe: &Path) -> Result<()> {
    let xyz = v.xyz();
    let target = paths.version_dir(v);
    // 旧版本若存在，先移到备份目录；安装成功后删除备份，失败则回滚，避免失败时旧版本丢失
    let backup = if target.exists() {
        let b = paths.versions().join(format!(".old-{}", v.canonical()));
        if b.exists() {
            std::fs::remove_dir_all(&b)?;
        }
        std::fs::rename(&target, &b)?;
        Some(b)
    } else {
        None
    };
    let log = paths.logs().join(format!("install-{xyz}.log"));
    let target_str = target.to_string_lossy().to_string();

    let args = vec![
        "/quiet".to_string(),
        "InstallAllUsers=0".to_string(),
        format!("TargetDir={target_str}"),
        "Include_launcher=0".to_string(),
        "Shortcuts=0".to_string(),
        "AssociateFiles=0".to_string(),
        "PrependPath=0".to_string(),
        "AppendPath=0".to_string(),
        "CompileAll=0".to_string(),
        "Include_test=0".to_string(),
        "Include_doc=0".to_string(),
        "Include_pip=1".to_string(),
        "Include_tcltk=1".to_string(),
    ];

    let mut cmd = Command::new(exe);
    cmd.args(&args).arg("/log").arg(&log);
    no_window(&mut cmd);
    let status = cmd.status();
    let result = match status {
        Ok(s) => match s.code() {
            Some(0) | Some(3010) => Ok(()),
            Some(code) => Err(PvmError::Installer {
                code,
                log: log.to_string_lossy().to_string(),
            }),
            None => Err(PvmError::Installer {
                code: -1,
                log: log.to_string_lossy().to_string(),
            }),
        },
        Err(e) => Err(PvmError::Installer {
            code: -1,
            log: format!("无法启动安装器: {e}"),
        }),
    };

    if let Some(b) = backup {
        if result.is_ok() {
            std::fs::remove_dir_all(&b).ok();
        } else {
            if target.exists() {
                std::fs::remove_dir_all(&target).ok();
            }
            std::fs::rename(&b, &target).ok();
        }
    }
    result
}

/// 卸载：用原 bootstrapper（缓存或重下）/quiet /uninstall，再删空目录。
pub fn uninstall_via_installer(v: &PythonVersion, paths: &Paths) -> Result<()> {
    let xyz = v.xyz();
    let exe = paths.cache().join(format!("python-{xyz}-amd64.exe"));
    let exe = if exe.exists() {
        exe
    } else {
        std::fs::create_dir_all(paths.cache())?;
        download_to(&DownloadOpts {
            url: &installer_url(v),
            dest: &exe,
            expect_sha256: None,
            quiet: false,
        })?;
        exe
    };

    let mut cmd = Command::new(&exe);
    cmd.args(["/quiet", "/uninstall"]);
    no_window(&mut cmd);
    let status = cmd.status().map_err(|e| PvmError::Installer {
        code: -1,
        log: format!("无法启动卸载器: {e}"),
    })?;
    match status.code() {
        Some(0) | Some(3010) => {}
        Some(code) => {
            return Err(PvmError::Installer {
                code,
                log: "卸载器返回非成功退出码，已保留版本目录".into(),
            })
        }
        None => {
            return Err(PvmError::Installer {
                code: -1,
                log: "卸载器被终止，已保留版本目录".into(),
            })
        }
    }

    let target = paths.version_dir(v);
    if target.exists() {
        std::fs::remove_dir_all(&target).ok();
    }
    Ok(())
}

/// 嵌入式安装（受限备选）：解压 zip → 改 ._pth 启用 site → get-pip.py。
pub fn install_via_embed(v: &PythonVersion, paths: &Paths) -> Result<()> {
    let xyz = v.xyz();
    std::fs::create_dir_all(paths.cache())?;
    std::fs::create_dir_all(paths.versions())?;

    let zipf = paths.cache().join(format!("python-{xyz}-embed-amd64.zip"));
    download_to(&DownloadOpts {
        url: &embed_url(&xyz),
        dest: &zipf,
        expect_sha256: None,
        quiet: false,
    })?;

    let tmp = paths.versions().join(format!(".tmp-{}", v.canonical()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    extract(&zipf, &tmp, ArchiveKind::Zip)?;

    patch_pth(&tmp)?;
    bootstrap_pip(&tmp, paths)?;

    let target = paths.version_dir(v);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

/// 取消 pythonXY._pth 中 `#import site` 的注释，启用 site（pip/venv 需要）。
fn patch_pth(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        if fname.starts_with("python") && fname.ends_with("._pth") {
            let content = std::fs::read_to_string(entry.path())?;
            let patched = content.replace("#import site", "import site");
            std::fs::write(entry.path(), patched)?;
        }
    }
    Ok(())
}

fn bootstrap_pip(dir: &Path, paths: &Paths) -> Result<()> {
    let getpip = paths.cache().join("get-pip.py");
    download_to(&DownloadOpts {
        url: "https://bootstrap.pypa.io/get-pip.py",
        dest: &getpip,
        expect_sha256: None,
        quiet: false,
    })?;
    let py = dir.join("python.exe");
    let mut cmd = Command::new(&py);
    cmd.arg(&getpip);
    no_window(&mut cmd);
    let status = cmd
        .status()
        .map_err(|e| PvmError::Http(format!("运行 get-pip.py 失败: {e}")))?;
    if !status.success() {
        return Err(PvmError::Archive("get-pip.py 执行失败".into()));
    }
    Ok(())
}
