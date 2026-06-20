//! pvm —— Windows 平台 Python 多版本管理器（核心库）。
//!
//! 实现按 docs/SPEC.md 分层推进：本阶段建立地基模块（error/version/paths/config），
//! 功能模块（resolve/download/archive/source_pbs/source_pyorg/shim/venv/pip/winpath）
//! 与命令层（commands/cli）随后逐步填充。

pub mod archive;
pub mod cli;
pub mod commands;
pub mod config;
pub mod download;
pub mod error;
pub mod paths;
pub mod pip;
pub mod resolve;
pub mod shim;
pub mod source_pbs;
pub mod source_pyorg;
pub mod venv;
pub mod version;
pub mod winpath;
