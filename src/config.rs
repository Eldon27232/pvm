//! 全局配置 config.toml。注意：**不存 global 版本**，全局版本以 root\version 文件为唯一权威。

use crate::error::{PvmError, Result};
use crate::paths::Paths;
use crate::version::Source;

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Config {
    /// 默认来源："standalone" | "cpython"；缺省视为 standalone。
    pub default_source: Option<String>,
    /// 默认 pip 镜像别名。
    pub pip_mirror: Option<String>,
    /// 关闭 shim 的 .python-version 自动切换（保留供高级用户）。
    #[serde(default)]
    pub disable_auto_switch: bool,
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.config_file();
        if !p.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&p)?;
        toml::from_str(&text).map_err(|e| PvmError::Config(format!("解析 config.toml 失败: {e}")))
    }

    /// 临时文件 + rename 原子写。
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let p = paths.config_file();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| PvmError::Config(format!("序列化 config 失败: {e}")))?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    pub fn default_source_resolved(&self) -> Source {
        self.default_source
            .as_deref()
            .and_then(Source::from_cli)
            .unwrap_or(Source::Standalone)
    }
}
