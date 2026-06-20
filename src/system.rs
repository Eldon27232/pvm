//! 发现 pvm 之外、系统上已安装的 Python（注册表 PythonCore + py launcher）。
//! 仅用于在界面中“识别现有环境”，这些解释器不纳入 pvm 的版本切换管理。

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

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

fn collect_from_launcher(out: &mut Vec<SystemPython>, seen: &mut HashSet<String>) {
    let output = match Command::new("py").arg("--list-paths").output() {
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
