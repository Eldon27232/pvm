//! 所有磁盘路径从 root（默认 %USERPROFILE%\.pvm）派生。
//!
//! 关键：`python_exe`/`scripts_dir` 按来源选择子路径——
//! standalone(install_only) 解包后多一层 `python\`，org 安装器直接装在版本目录根。

use crate::error::{PvmError, Result};
use crate::version::{PythonVersion, Source};
use std::path::PathBuf;

pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// 解析根目录：`--root` > `PVM_ROOT` > `%USERPROFILE%\.pvm`。
    pub fn discover(root_override: Option<PathBuf>) -> Result<Self> {
        if let Some(r) = root_override {
            return Ok(Self { root: r });
        }
        if let Ok(env_root) = std::env::var("PVM_ROOT") {
            if !env_root.trim().is_empty() {
                return Ok(Self {
                    root: PathBuf::from(env_root),
                });
            }
        }
        let base = directories::BaseDirs::new()
            .ok_or_else(|| PvmError::Config("无法定位用户主目录".into()))?;
        Ok(Self {
            root: base.home_dir().join(".pvm"),
        })
    }

    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }
    pub fn shims(&self) -> PathBuf {
        self.root.join("shims")
    }
    pub fn venvs(&self) -> PathBuf {
        self.root.join("venvs")
    }
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn backup(&self) -> PathBuf {
        self.root.join("backup")
    }
    /// root\bin —— shim 模板（pvm-shim.exe / pvm-shimw.exe）所在。
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    /// 全局默认版本文件（单行 canonical id），全局版本的唯一权威。
    pub fn global_version_file(&self) -> PathBuf {
        self.root.join("version")
    }
    pub fn version_dir(&self, v: &PythonVersion) -> PathBuf {
        self.versions().join(v.canonical())
    }

    /// 真实 python.exe 路径，按来源区分。
    pub fn python_exe(&self, v: &PythonVersion) -> PathBuf {
        let d = self.version_dir(v);
        match v.source {
            Source::Standalone => d.join("python").join("python.exe"),
            Source::Org => d.join("python.exe"),
        }
    }

    /// pythonw.exe（与 python.exe 同目录）。
    pub fn pythonw_exe(&self, v: &PythonVersion) -> PathBuf {
        self.python_exe(v).with_file_name("pythonw.exe")
    }

    /// Scripts 目录（pip.exe 及 entry-point 所在）。
    pub fn scripts_dir(&self, v: &PythonVersion) -> PathBuf {
        let d = self.version_dir(v);
        match v.source {
            Source::Standalone => d.join("python").join("Scripts"),
            Source::Org => d.join("Scripts"),
        }
    }
}
