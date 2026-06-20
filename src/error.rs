//! 统一错误类型。每个变体与退出码、用户可读中文信息一一对应，禁止用笼统错误替代语义错误。

use thiserror::Error;

#[derive(Error, Debug)]
pub enum PvmError {
    #[error("io 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("网络/下载错误: {0}")]
    Http(String),

    #[error("GitHub API 限流(匿名 60/h)，将在 epoch {reset} 重置；可设置 GITHUB_TOKEN 提升到 5000/h")]
    RateLimited { reset: String },

    #[error("解压错误: {0}")]
    Archive(String),

    #[error("SHA256 校验失败: 期望 {expected}, 实际 {actual}")]
    Checksum { expected: String, actual: String },

    #[error("未找到版本: {0}")]
    VersionNotFound(String),

    #[error("版本未安装: {0}")]
    NotInstalled(String),

    #[error("版本来源有歧义，请用 @standalone/@org 或 --source 指定。候选: {0}")]
    Ambiguous(String),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("未设置 Python 版本（无 PVM_VERSION、无 .python-version、无 global）")]
    NoVersionConfigured,

    #[error("安装器失败({code})，详见日志: {log}")]
    Installer { code: i32, log: String },

    #[error("Windows 系统调用失败: {0}")]
    Win(String),

    #[error("用法错误: {0}")]
    Usage(String),
}

pub type Result<T> = std::result::Result<T, PvmError>;

impl PvmError {
    /// 进程退出码映射（见 SPEC §3.1）。
    /// 4=网络/限流，5=校验失败，3=未安装/未找到，其余=1。
    pub fn exit_code(&self) -> i32 {
        match self {
            PvmError::Usage(_) => 2,
            PvmError::Http(_) | PvmError::RateLimited { .. } => 4,
            PvmError::Checksum { .. } => 5,
            PvmError::NotInstalled(_) | PvmError::VersionNotFound(_) => 3,
            _ => 1,
        }
    }
}
