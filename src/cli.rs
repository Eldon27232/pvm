//! clap 命令行定义。

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pvm", version, about = "Windows 平台 Python 多版本管理器")]
pub struct Cli {
    /// 覆盖 pvm 根目录（默认 %USERPROFILE%\.pvm）
    #[arg(long, global = true, env = "PVM_ROOT")]
    pub root: Option<PathBuf>,
    /// 增加日志详细度（可叠加）
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// 仅输出错误
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,
    /// 结构化输出（部分命令）
    #[arg(long, global = true)]
    pub json: bool,
    /// 跳过交互确认
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(ValueEnum, Clone, Copy, PartialEq, Eq)]
pub enum SourceArg {
    Standalone,
    Cpython,
}

#[derive(ValueEnum, Clone, Copy)]
pub enum FlavorArg {
    #[value(name = "install_only")]
    InstallOnly,
    #[value(name = "stripped")]
    Stripped,
    #[value(name = "full")]
    Full,
}

#[derive(ValueEnum, Clone, Copy)]
pub enum ScopeArg {
    Global,
    Venv,
}

impl SourceArg {
    pub fn to_source(self) -> crate::version::Source {
        match self {
            SourceArg::Standalone => crate::version::Source::Standalone,
            SourceArg::Cpython => crate::version::Source::Org,
        }
    }
}

impl FlavorArg {
    pub fn to_flavor(self) -> crate::source_pbs::PbsFlavor {
        match self {
            FlavorArg::InstallOnly => crate::source_pbs::PbsFlavor::InstallOnly,
            FlavorArg::Stripped => crate::source_pbs::PbsFlavor::InstallOnlyStripped,
            FlavorArg::Full => crate::source_pbs::PbsFlavor::PgoFull,
        }
    }
}

impl ScopeArg {
    pub fn to_scope(self) -> crate::pip::Scope {
        match self {
            ScopeArg::Global => crate::pip::Scope::Global,
            ScopeArg::Venv => crate::pip::Scope::Venv,
        }
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// 安装一个或多个 Python 版本
    #[command(visible_alias = "i")]
    Install {
        versions: Vec<String>,
        #[arg(long, value_enum)]
        source: Option<SourceArg>,
        #[arg(long, value_enum, default_value_t = FlavorArg::InstallOnly)]
        flavor: FlavorArg,
        #[arg(long)]
        freethreaded: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        skip_existing: bool,
        #[arg(short = 'g', long = "set-global")]
        set_global: bool,
        #[arg(long)]
        no_verify: bool,
        #[arg(long)]
        mirror: Option<String>,
    },
    /// 卸载版本
    #[command(visible_aliases = ["rm", "remove"])]
    Uninstall {
        versions: Vec<String>,
        #[arg(long)]
        keep_venvs: bool,
    },
    /// 列出已安装版本（--remote 列远程）
    #[command(visible_alias = "ls")]
    List {
        #[arg(long)]
        remote: bool,
        #[arg(long, value_enum)]
        source: Option<SourceArg>,
        #[arg(long)]
        bare: bool,
        #[arg(long)]
        all: bool,
    },
    /// 列出远程可安装版本
    #[command(name = "ls-remote")]
    LsRemote {
        #[arg(long, value_enum)]
        source: Option<SourceArg>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        refresh: bool,
    },
    /// 设置/查看全局默认版本
    #[command(visible_alias = "use")]
    Global { version: Option<String> },
    /// 设置/查看当前目录 .python-version
    Local {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },
    /// 查看/提示会话级 PVM_VERSION
    Shell {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },
    /// 打印 shim 解析到的真实 exe
    Which { exe: Option<String> },
    /// 打印当前生效版本与来源
    #[command(visible_alias = "version")]
    Current,
    /// 用指定/生效版本运行命令
    #[command(visible_alias = "run")]
    Exec {
        #[arg(long)]
        version: Option<String>,
        #[arg(last = true)]
        cmd: Vec<String>,
    },
    /// 虚拟环境管理
    Venv {
        #[command(subcommand)]
        cmd: VenvCmd,
    },
    /// pip 镜像配置
    #[command(name = "pip-mirror", visible_alias = "mirror")]
    PipMirror {
        #[command(subcommand)]
        cmd: MirrorCmd,
    },
    /// 初始化：建目录、装 shim、把 shims 加入 PATH
    Init {
        #[arg(long)]
        path_only: bool,
    },
    /// 重建 shim
    Rehash,
    /// 环境诊断
    Doctor,
    /// 打印 pvm 根目录
    Root,
}

#[derive(Subcommand)]
pub enum VenvCmd {
    /// 创建 venv
    Create {
        name: String,
        #[arg(long = "python", visible_alias = "version")]
        python: Option<String>,
        #[arg(long)]
        in_project: bool,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        without_pip: bool,
        #[arg(long)]
        system_site_packages: bool,
        #[arg(long)]
        mirror: Option<String>,
    },
    /// 列出集中式 venv
    #[command(visible_alias = "ls")]
    List,
    /// 删除 venv
    #[command(visible_alias = "rm")]
    Remove { name: String },
    /// 打印 venv 路径
    Path { name: String },
    /// 打印 venv 的 python.exe
    Which { name: String },
    /// 打印激活命令
    Activate { name: String },
}

#[derive(Subcommand)]
pub enum MirrorCmd {
    /// 设置镜像（内置别名或裸 URL）
    Set {
        name_or_url: String,
        #[arg(long, value_enum, default_value_t = ScopeArg::Global)]
        scope: ScopeArg,
        #[arg(long)]
        no_trusted_host: bool,
    },
    /// 显示当前镜像
    Show {
        #[arg(long, value_enum, default_value_t = ScopeArg::Global)]
        scope: ScopeArg,
    },
    /// 列出内置镜像
    List,
    /// 重置镜像
    Reset {
        #[arg(long, value_enum, default_value_t = ScopeArg::Global)]
        scope: ScopeArg,
    },
}
