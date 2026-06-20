//! pip 镜像配置：写 pip.ini 的 [global] index-url，支持全局与 venv 作用域。
//! 注意：基于 rust-ini 重写整个文件，保留键值但会丢弃原有注释/空行；首次写入前备份为 pip.ini.pvm.bak。

use crate::error::{PvmError, Result};
use crate::paths::Paths;
use ini::Ini;
use std::path::{Path, PathBuf};

pub struct Mirror {
    pub aliases: &'static [&'static str],
    pub display: &'static str,
    pub index_url: &'static str,
    pub trusted_host: Option<&'static str>,
}

/// 内置镜像表（已核实，2026-06）。
pub const MIRRORS: &[Mirror] = &[
    Mirror {
        aliases: &["tuna", "tsinghua"],
        display: "清华 TUNA",
        index_url: "https://pypi.tuna.tsinghua.edu.cn/simple",
        trusted_host: Some("pypi.tuna.tsinghua.edu.cn"),
    },
    Mirror {
        aliases: &["aliyun", "ali"],
        display: "阿里云",
        index_url: "https://mirrors.aliyun.com/pypi/simple/",
        trusted_host: Some("mirrors.aliyun.com"),
    },
    Mirror {
        aliases: &["ustc"],
        display: "中科大",
        index_url: "https://mirrors.ustc.edu.cn/pypi/simple",
        trusted_host: Some("mirrors.ustc.edu.cn"),
    },
    Mirror {
        aliases: &["tencent", "qcloud"],
        display: "腾讯云",
        index_url: "https://mirrors.cloud.tencent.com/pypi/simple",
        trusted_host: Some("mirrors.cloud.tencent.com"),
    },
    Mirror {
        aliases: &["huawei", "hwcloud"],
        display: "华为云",
        index_url: "https://repo.huaweicloud.com/repository/pypi/simple",
        trusted_host: Some("repo.huaweicloud.com"),
    },
    Mirror {
        aliases: &["pypi", "official"],
        display: "官方源",
        index_url: "https://pypi.org/simple",
        trusted_host: None,
    },
];

#[derive(Clone, Copy)]
pub enum Scope {
    Global,
    Venv,
}

pub fn lookup_mirror(key: &str) -> Option<&'static Mirror> {
    let k = key.to_ascii_lowercase();
    MIRRORS
        .iter()
        .find(|m| m.aliases.iter().any(|a| *a == k))
}

pub fn pip_ini_path(scope: Scope, venv: Option<&Path>, _paths: &Paths) -> Result<PathBuf> {
    match scope {
        Scope::Global => {
            let appdata = std::env::var("APPDATA")
                .map_err(|_| PvmError::Config("未找到 APPDATA 环境变量".into()))?;
            Ok(PathBuf::from(appdata).join("pip").join("pip.ini"))
        }
        Scope::Venv => {
            let v = venv.ok_or_else(|| PvmError::Config("venv 作用域需指定 venv 路径".into()))?;
            Ok(v.join("pip.ini"))
        }
    }
}

/// 设置镜像：name_or_url 为内置别名或裸 http(s) URL。
pub fn pip_mirror_set(
    name_or_url: &str,
    scope: Scope,
    venv: Option<&Path>,
    no_trusted: bool,
    paths: &Paths,
) -> Result<()> {
    let (index_url, trusted): (String, Option<String>) = match lookup_mirror(name_or_url) {
        Some(m) => (
            m.index_url.to_string(),
            m.trusted_host.map(|s| s.to_string()),
        ),
        None => {
            let u = url::Url::parse(name_or_url)
                .map_err(|_| PvmError::Config(format!("无效镜像地址: {name_or_url}")))?;
            if u.scheme() != "http" && u.scheme() != "https" {
                return Err(PvmError::Config("镜像地址需为 http(s) 协议".into()));
            }
            let host = u.host_str().map(|s| s.to_string());
            (name_or_url.to_string(), host)
        }
    };

    let path = pip_ini_path(scope, venv, paths)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let bak = path.with_extension("ini.pvm.bak");
        if !bak.exists() {
            std::fs::copy(&path, &bak)?;
        }
    }

    let mut conf = if path.exists() {
        Ini::load_from_file(&path)
            .map_err(|e| PvmError::Config(format!("读取 pip.ini 失败: {e}")))?
    } else {
        Ini::new()
    };

    conf.with_section(Some("global"))
        .set("index-url", index_url);

    let set_trusted = !no_trusted;
    if set_trusted {
        if let Some(h) = trusted {
            conf.with_section(Some("global")).set("trusted-host", h);
        } else if let Some(sec) = conf.section_mut(Some("global")) {
            sec.remove("trusted-host");
        }
    } else if let Some(sec) = conf.section_mut(Some("global")) {
        sec.remove("trusted-host");
    }

    conf.write_to_file(&path)
        .map_err(|e| PvmError::Config(format!("写入 pip.ini 失败: {e}")))?;
    Ok(())
}

/// 仅删除 pvm 写入的 index-url/trusted-host，不删整文件。
pub fn pip_mirror_reset(scope: Scope, venv: Option<&Path>, paths: &Paths) -> Result<()> {
    let path = pip_ini_path(scope, venv, paths)?;
    if !path.exists() {
        return Ok(());
    }
    let mut conf = Ini::load_from_file(&path)
        .map_err(|e| PvmError::Config(format!("读取 pip.ini 失败: {e}")))?;
    if let Some(sec) = conf.section_mut(Some("global")) {
        sec.remove("index-url");
        sec.remove("trusted-host");
    }
    conf.write_to_file(&path)
        .map_err(|e| PvmError::Config(format!("写入 pip.ini 失败: {e}")))?;
    Ok(())
}

pub fn pip_mirror_show(scope: Scope, venv: Option<&Path>, paths: &Paths) -> Result<()> {
    let path = pip_ini_path(scope, venv, paths)?;
    if !path.exists() {
        println!("（未配置 pip 镜像，使用 pip 默认源）");
        return Ok(());
    }
    let conf = Ini::load_from_file(&path)
        .map_err(|e| PvmError::Config(format!("读取 pip.ini 失败: {e}")))?;
    let idx = conf.get_from(Some("global"), "index-url").unwrap_or("(未设置)");
    let th = conf
        .get_from(Some("global"), "trusted-host")
        .unwrap_or("(无)");
    println!("pip.ini: {}", path.display());
    println!("  index-url    = {idx}");
    println!("  trusted-host = {th}");
    Ok(())
}

/// 列出内置镜像。
pub fn pip_mirror_list() {
    println!("内置 pip 镜像：");
    for m in MIRRORS {
        println!(
            "  {:10} {}  ->  {}",
            m.aliases[0],
            m.display,
            m.index_url
        );
    }
}
