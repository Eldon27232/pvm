//! 发现 pvm 之外、系统上已安装的 Python（注册表 PythonCore + py launcher）。
//! 仅用于在界面中“识别现有环境”，这些解释器不纳入 pvm 的版本切换管理。

use std::collections::HashSet;
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

#[derive(serde::Serialize, Clone, Debug)]
pub struct SystemPython {
    /// 版本号（尽量取到 x.y.z，最少 x.y）。
    pub version: String,
    /// python.exe 绝对路径。
    pub path: String,
    /// 来源：registry / launcher。
    pub origin: String,
}

/// 扫描系统已安装的 Python（注册表 + py launcher），按可执行路径去重。
pub fn list_system_pythons() -> Vec<SystemPython> {
    let mut out: Vec<SystemPython> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    #[cfg(windows)]
    collect_from_registry(&mut out, &mut seen);
    collect_from_launcher(&mut out, &mut seen);
    out
}

#[cfg(windows)]
fn collect_from_registry(out: &mut Vec<SystemPython>, seen: &mut HashSet<String>) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let subs = [
        "SOFTWARE\\Python\\PythonCore",
        "SOFTWARE\\WOW6432Node\\Python\\PythonCore",
    ];
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root = RegKey::predef(hive);
        for sub in subs {
            let pc = match root.open_subkey(sub) {
                Ok(k) => k,
                Err(_) => continue,
            };
            for tag in pc.enum_keys().flatten() {
                let tagkey = match pc.open_subkey(&tag) {
                    Ok(k) => k,
                    Err(_) => continue,
                };
                let ip = match tagkey.open_subkey("InstallPath") {
                    Ok(k) => k,
                    Err(_) => continue,
                };
                let exe: Option<String> = ip.get_value("ExecutablePath").ok();
                let dir: Option<String> = ip.get_value("").ok();
                let exe_path = exe.or_else(|| {
                    dir.map(|d| format!("{}\\python.exe", d.trim_end_matches('\\')))
                });
                if let Some(p) = exe_path {
                    if Path::new(&p).exists() && seen.insert(p.to_lowercase()) {
                        let ver: Option<String> = tagkey.get_value("Version").ok();
                        let sysver: Option<String> = tagkey.get_value("SysVersion").ok();
                        out.push(SystemPython {
                            version: ver.or(sysver).unwrap_or_else(|| tag.clone()),
                            path: p,
                            origin: "registry".into(),
                        });
                    }
                }
            }
        }
    }
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct PathPython {
    pub path: String,
    pub dir: String,
    pub fake: bool,
    pub effective: bool,
}

/// 解析 PATH，列出会被命令行 `python` 命中的 exe（按顺序），标记 WindowsApps 假 python 与生效项。
pub fn path_pythons() -> Vec<PathPython> {
    let mut out: Vec<PathPython> = Vec::new();
    let path = std::env::var("PATH").unwrap_or_default();
    let mut seen = HashSet::new();
    for dir in path.split(';') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        let p = Path::new(dir).join("python.exe");
        if p.is_file() {
            let key = p.to_string_lossy().to_lowercase();
            if !seen.insert(key) {
                continue;
            }
            let fake = dir.to_lowercase().contains("windowsapps");
            out.push(PathPython {
                path: p.to_string_lossy().to_string(),
                dir: dir.to_string(),
                fake,
                effective: false,
            });
        }
    }
    // PATH 中第一个 python.exe 会被真正调用（即使是 WindowsApps 的 stub）。
    if let Some(first) = out.first_mut() {
        first.effective = true;
    }
    out
}

fn collect_from_launcher(out: &mut Vec<SystemPython>, seen: &mut HashSet<String>) {
    let mut cmd = Command::new("py");
    cmd.arg("--list-paths");
    no_window(&mut cmd); // GUI 无 console，避免 py launcher 闪现 cmd 窗口
    let output = match cmd.output() {
        Ok(o) if o.status.success() => o,
        _ => return,
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let path_re = match regex::Regex::new(r"(?i)([A-Z]:\\[^\r\n]*?python\.exe)") {
        Ok(r) => r,
        Err(_) => return,
    };
    let ver_re = regex::Regex::new(r"-V:(\d+\.\d+(?:\.\d+)?)").ok();
    for line in text.lines() {
        if let Some(c) = path_re.captures(line) {
            let p = c[1].trim().to_string();
            if Path::new(&p).exists() && seen.insert(p.to_lowercase()) {
                let ver = ver_re
                    .as_ref()
                    .and_then(|r| r.captures(line))
                    .map(|c| c[1].to_string())
                    .unwrap_or_else(|| "?".into());
                out.push(SystemPython {
                    version: ver,
                    path: p,
                    origin: "launcher".into(),
                });
            }
        }
    }
}
