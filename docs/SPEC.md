I have verified the load-bearing facts. Key corrections to fold in:

1. **PBS full flavor suffix** is `pgo-full.tar.zst` (not generic `full`); the SPEC's `PbsFlavor::Full.suffix()` must return `pgo-full.tar.zst`.
2. **PBS freethreaded Windows naming** uses a compound `freethreaded+pgo-install_only.tar.gz` form, not a flat `freethreaded-install_only.tar.gz` — must mark the exact suffix as 待定/regex-driven, not hardcoded.
3. **python.org installer**: `Include_launcher=0` is valid but the 3.13+ installer ships the new "Python install manager" as the launcher; per-user silent install with custom TargetDir is sound, exit codes (0/3010/1602/1638/1603) are standard MSI conventions — keep but mark uninstaller-via-cached-exe as the practical path. `DefaultJustForMeTargetDir` exists.
4. Everything else (digest `sha256:` prefix, ftp URL template, windows-sys feature names, crate versions) checks out.

Now producing the final revised SPEC.

---

# PVM 技术规格说明书（SPEC）· 修订最终版

> Windows 平台 Python 多版本管理器 · Rust 实现 · 单一权威规格
> 本文综合 6 个子系统设计，解决其间命名、接口、来源标识、默认策略的冲突，作为 Rust 实现的唯一依据。
> 修订说明：已抽查核实 PBS 资产命名（`cpython-<X.Y.Z>+<YYYYMMDD>-x86_64-pc-windows-msvc-install_only.tar.gz`、`digest: "sha256:..."`、full 实为 `pgo-full.tar.zst`）、python.org 安装器参数与 ftp URL 模板（`python-<X.Y.Z>-amd64.exe`）、`windows-sys 0.59` feature 名、各 crate 版本。修订点已就地标注，仍存疑处用「> 待定:」标注。

---

## 1. 项目概述与设计目标

### 1.1 定位

`pvm` 是一个**仅面向 Windows x86_64** 的命令行 Python 版本管理器，对标 pyenv-win，但在四个方向上做了明确增强：

1. **shim 用原生 Rust exe 而非 .bat**，真正实现按目录读取 `.python-version` 的确定性自动切换、Ctrl+C 透传、退出码透传。
2. **双解释器来源**：`python-build-standalone`（默认）与 `python.org` 官方安装器，同一版本号双来源可并存。
3. **内建 venv 子系统与 pip 国内镜像子系统**。
4. **直连远端实时枚举可装版本**（GitHub Releases / python.org），全查询命令支持 `--json`。

### 1.2 设计目标与原则

- **无需管理员权限**：所有写操作限于用户级（`%USERPROFILE%\.pvm`、`HKCU\Environment`、`%APPDATA%\pip`）。
- **零系统污染**：不改系统 PATH、不写 HKLM、不建快捷方式、不关联文件。
- **确定性优先**：版本解析顺序固定（shell > local > global）；`local`/`global` 落盘写 canonical id 保证可复现。
- **最小依赖、可单 exe 分发**：默认只走纯 Rust 解压路径（`.tar.gz`），`+crt-static` 静态链接 CRT，主程序与 shim 拆分为多个二进制。
- **不吞错**：网络、校验、解压、子进程退出码全部显式传播，错误信息中文、可操作。

### 1.3 不支持范围（明确边界）

- 不支持 32 位、ARM64（只取 `x86_64-pc-windows-msvc` / `-amd64`）。
- 不支持 Linux/macOS（venv 脚本目录固定 `Scripts\`，shim 用 Win32 API）。
- 不支持 PyPy/GraalPy（canonical id 用 `cpython-` 前缀预留命名空间，但本期不实现）。

---

## 2. 磁盘目录布局（`.pvm` 下结构）

根目录默认 `%USERPROFILE%\.pvm`，可被 `--root` flag 或 `PVM_ROOT` 环境变量覆盖。

```
%USERPROFILE%\.pvm\
├─ version                         # 全局默认版本，单行 canonical id，pvm global 写入
├─ config.toml                     # 全局配置（默认来源、pip 镜像、行为开关）
├─ bin\
│   ├─ pvm.exe                     # 主程序（也可在 PATH 任意位置；rehash 从此处或已知路径取 shim 模板）
│   ├─ pvm-shim.exe                # shim 模板（console 子系统）
│   └─ pvm-shimw.exe               # shim 模板（windows/GUI 子系统，供 pythonw 用）
├─ shims\                          # 此目录写入用户 PATH 最前
│   ├─ python.exe                  # = pvm-shim.exe 的副本
│   ├─ pythonw.exe                 # = pvm-shimw.exe 的副本（GUI 子系统）
│   ├─ pip.exe  pip3.exe
│   └─ <venv/Scripts 暴露的入口>.exe
├─ versions\
│   ├─ cpython-3.12.7-standalone\  # python-build-standalone 来源
│   │   └─ python\python.exe       # 注意：standalone install_only 解包后多一层 python\
│   │       └─ Scripts\pip.exe
│   ├─ cpython-3.12.7-org\         # python.org 官方安装器来源
│   │   ├─ python.exe              # 注意：org 来源解释器直接在版本目录根（=TargetDir）
│   │   └─ Scripts\pip.exe
│   └─ cpython-3.13.14-standalone\
├─ venvs\
│   └─ <name>\                     # 集中式 venv
│       ├─ Scripts\python.exe
│       ├─ pip.ini                 # 可选，venv 级 pip 镜像
│       └─ .pvm-venv.json          # venv 元数据
├─ cache\
│   ├─ pbs-index.json              # PBS 远端枚举缓存（带 ETag）
│   ├─ pyorg-index.json            # python.org 远端枚举缓存（带 TTL）
│   ├─ <下载的归档/安装器>          # *.tar.gz / python-<ver>-amd64.exe（卸载需保留）
│   └─ *.part                      # 下载中的临时文件
├─ logs\
│   └─ install-<ver>.log           # python.org 安装器 /log 输出
└─ backup\
    └─ user-path.bak               # 首次改 PATH 前备份原始 Path 值
```

**关键路径差异（实现必须区分两来源）：**

| 来源 | canonical id 后缀 | 真实 `python.exe` 相对版本目录的路径 |
|---|---|---|
| python-build-standalone（install_only） | `-standalone` | `python\python.exe` |
| python.org 官方安装器 | `-org` | `python.exe`（直接在 TargetDir 根） |

`Paths::python_exe(v)` 必须按 `v.source` 选择正确子路径。

> 已核实：PBS `install_only` 归档解包后顶层是 `python\`（内含 `python.exe`、`Scripts\`、`Lib\` 等）。`pgo-full.tar.zst`（full）解包顶层为 `python\install\`，本期 full 路径仅在启用 `zstd-full` feature 时支持，且需在 `install_pbs` 中按 flavor 选择校验子路径。

---

## 3. 完整命令表

### 3.1 全局 flag（对所有子命令生效）

| flag | 别名 | 说明 |
|---|---|---|
| `--help` | `-h` | 帮助；`pvm help <cmd>` 等价 |
| `--version` | `-V` | 打印 pvm 自身版本 |
| `--verbose` | `-v` | 可叠加 `-vv` 提升日志等级 |
| `--quiet` | `-q` | 仅输出错误，抑制进度条 |
| `--root <DIR>` | | 覆盖 pvm 根目录，亦可用 `PVM_ROOT` |
| `--color <WHEN>` | | `auto`(默认)/`always`/`never`，遵守 `NO_COLOR` |
| `--json` | | 结构化输出（对有结构化输出的命令有效） |
| `--yes` | `-y` | 跳过交互确认 |

**配置优先级**：CLI flag > 环境变量（`PVM_ROOT` / `PVM_VERSION` / `PVM_DEFAULT_SOURCE` / `PVM_DISABLE_AUTO_SWITCH` / `GITHUB_TOKEN`）> `config.toml` > 内置默认。

**退出码枚举**：`0` 成功；`1` 一般错误；`2` 用法/参数错误（clap 默认）；`3` 版本未安装；`4` 网络/下载失败；`5` 校验失败（哈希）。shim 专用：`126` 启动子进程失败；`127` 版本解析失败/未配置。

### 3.2 子命令

| 命令 | 别名 | 用法 / 参数 | 说明 | 示例 |
|---|---|---|---|---|
| `install` | `i` | `pvm install <VERSION>... [--source standalone\|cpython] [--flavor install_only\|stripped\|full] [--freethreaded] [--force] [--skip-existing] [--set-global/-g] [--no-verify] [--mirror <URL>]` | 安装一个或多个版本，装完自动 rehash | `pvm install 3.12 -g`；`pvm install 3.12.7 --source cpython` |
| `uninstall` | `rm`,`remove` | `pvm uninstall <VERSION>... [-y] [--keep-venvs]` | 卸载；org 来源调原安装器卸载，standalone 删目录；之后 rehash | `pvm uninstall 3.12.7@org -y` |
| `list` | `ls` | `pvm list [--remote] [--source <s>] [--bare] [--all] [--json]` | 列本地，`--remote` 转列远程 | `pvm list`；`pvm ls --bare` |
| `ls-remote` | | `pvm ls-remote [--source <s>] [--all] [--refresh] [--json]` | 列远程可装版本（顶层别名，等价 `list --remote`） | `pvm ls-remote --source standalone` |
| `global` | `use` | `pvm global [<VERSION>]` | 无参打印当前全局；有参写 `.pvm\version` 并 rehash | `pvm global 3.12` |
| `local` | | `pvm local [<VERSION>] [--unset]` | 无参打印当前 local 及来源文件；有参写当前目录 `.python-version` | `pvm local 3.11`；`pvm local --unset` |
| `shell` | | `pvm shell [<VERSION>] [--unset]` | 设置/打印 `PVM_VERSION`（当前会话临时覆盖，依赖 init 注入的 shell 函数） | `pvm shell 3.13@standalone` |
| `which` | | `pvm which [<EXE>]` | 打印 shim 解析到的真实 exe 绝对路径（默认 python） | `pvm which pip` |
| `current` | `version` | `pvm current` | 打印当前生效版本 id 及来源 | `pvm current` |
| `exec` | `run` | `pvm exec [--version <V>] -- <CMD> [ARGS]...` | 不污染 PATH，临时用某版本跑命令 | `pvm exec --version 3.11 -- python -m pytest` |
| `venv create` | | `pvm venv create <NAME> [--python/--version <V>] [--in-project] [--path <DIR>] [--clear] [--without-pip] [--system-site-packages] [--mirror <name\|url>]` | 用选定版本 `python -m venv` 创建 | `pvm venv create web --version 3.12 --mirror tsinghua` |
| `venv list` | `ls` | `pvm venv list [--json]` | 列集中式 venv（不含 `--in-project` 的 `.venv`） | |
| `venv remove` | `rm` | `pvm venv remove <NAME> [-y]` | 删除集中式 venv | |
| `venv path` | | `pvm venv path <NAME>` | 打印 venv 根路径 | |
| `venv which` | | `pvm venv which <NAME>` | 打印 `<venv>\Scripts\python.exe` | |
| `venv activate` | | `pvm venv activate <NAME>` | 打印激活命令（不代替用户 source） | |
| `pip-mirror set` | `mirror` | `pvm pip-mirror set <name\|url> [--scope global\|venv] [--no-trusted-host] [--env]` | 写 pip.ini 的 `[global] index-url` | `pvm pip-mirror set tsinghua` |
| `pip-mirror show` | | `pvm pip-mirror show [--scope global\|venv]` | 显示当前生效 index-url/trusted-host | |
| `pip-mirror list` | | `pvm pip-mirror list` | 列内置别名及 URL | |
| `pip-mirror reset` | `unset` | `pvm pip-mirror reset [--scope global\|venv]` | 复位官方源（仅删 pvm 写入项） | |
| `init` | | `pvm init [--path-only] [--shell powershell\|cmd\|bash]` | 把 shims 写入用户 PATH 并广播；输出 shell 函数片段 | `pvm init` |
| `rehash` | | `pvm rehash` | 重建 shims | |
| `self` | | `pvm self version\|update\|uninstall [--purge]\|dir` | pvm 自身维护 | `pvm self update` |
| `root` | | `pvm root` | 打印根目录（= `pvm self dir`） | |
| `doctor` | | `pvm doctor` | 自检 PATH/shims/global-local 有效性/网络可达性 | |

> **命名一致性裁决**：子命令统一用连字符形式 `pip-mirror`（不用 `pip mirror` 两级子命令），动作为其下子命令 `set/show/list/reset`。`pip-mirror reset` 是权威动词，`unset` 为别名。
> **`--flavor` 取值对齐**：CLI `--flavor` 三值 `install_only`/`stripped`/`full` 与内部 `PbsFlavor` 的映射见 §4.3 与 §7.2；`--freethreaded` 与 `--flavor` 的组合约束见 §8.2。`install`/`uninstall`/`exec --version` 接受多来源歧义裁决（§8.2）。

---

## 4. 模块划分与接口契约

### 4.1 工程布局（单 crate、三 `[[bin]]`）

```
pvm/
├─ Cargo.toml                # [package] + [[bin]] pvm + [[bin]] pvm-shim + [[bin]] pvm-shimw
├─ build.rs                  # 注入 Windows manifest：longPathAware=true，requestedExecutionLevel=asInvoker
├─ .cargo/config.toml        # target-feature=+crt-static
├─ src/
│  ├─ main.rs                # pvm.exe 入口
│  ├─ bin/
│  │   ├─ shim.rs            # pvm-shim.exe（console 子系统）
│  │   └─ shimw.rs           # pvm-shimw.exe（#![windows_subsystem="windows"]）
│  ├─ cli.rs                 # clap derive 定义 + dispatch
│  ├─ commands/              # 薄编排层，每子命令一文件
│  │   ├─ mod.rs install.rs uninstall.rs list.rs global.rs local.rs
│  │   ├─ shell.rs which.rs current.rs exec.rs venv.rs pip_mirror.rs
│  │   ├─ init.rs rehash.rs self_cmd.rs doctor.rs
│  └─ core/
│      ├─ mod.rs
│      ├─ error.rs           # PvmError + Result
│      ├─ paths.rs           # Paths
│      ├─ config.rs          # Config
│      ├─ version.rs         # PythonVersion / Source / VersionSelector / 解析
│      ├─ resolve.rs         # 生效版本解析（shim 与 current 共用）
│      ├─ registry.rs        # 本地已装枚举 + 远端枚举聚合
│      ├─ source_pbs.rs      # python-build-standalone 来源子系统
│      ├─ source_pyorg.rs    # python.org 来源子系统
│      ├─ download.rs        # 下载 + SHA256 校验
│      ├─ archive.rs         # 解压分派 .tar.gz/.tar.zst/.zip
│      ├─ install.rs         # 安装编排
│      ├─ shim.rs            # 生成/重建 shims
│      ├─ venv.rs            # venv 创建/列举/移除
│      ├─ pip.rs             # pip.ini 镜像读写
│      └─ winpath.rs         # 长路径前缀、HKCU PATH 读写广播、Ctrl+C handler、Job Object
```

> **裁决（与草案统一）**：`core` 模块被三个 `[[bin]]` 共用。为避免重复编译核心逻辑，将 `core` 作为 crate 内的 `lib`（`src/lib.rs` 暴露 `pub mod core;` 与 `pub mod cli;`），三个 bin 均 `use pvm::core::*;`。即 `Cargo.toml` 含一个隐式 `[lib]` + 三个 `[[bin]]`。shim 二进制应通过 cfg/feature 仅链接 `core::{resolve, paths, version, error, winpath, config}` 所需子集，避免把 ureq/zip/zstd 等重依赖拖入 shim（shim 不发网络请求、不解压）。

### 4.2 各模块职责（一句话）

- **main.rs**：解析 argv → `cli::run()` → 把 `anyhow::Result` 渲染为中文错误并设退出码。
- **bin/shim.rs / shimw.rs**：极小转发器，由自身文件名判断命令，复用 `core::resolve`/`core::paths` 找真实 exe，启动子进程透传退出码与信号。
- **cli.rs**：clap derive 全部子命令；dispatch 到 `commands::*`。
- **commands/***：只组合 core 能力，不含核心逻辑。
- **core::error**：`PvmError` + `Result`。
- **core::paths**：从根计算所有子路径，含两来源 `python_exe` 差异。
- **core::config**：读写 `config.toml`，原子写。
- **core::version**：`PythonVersion`/`Source`/`VersionSelector` 类型与解析、语义排序、部分版本匹配。
- **core::resolve**：生效版本解析（shell→local→global），shim 与 `pvm current` 共用。
- **core::registry**：扫 `versions\` 列本地；调两个 source 模块聚合远端。
- **core::source_pbs / source_pyorg**：各自负责枚举、下载 URL 构造、安装、卸载。
- **core::download / archive / install / shim / venv / pip / winpath**：见各自接口。

### 4.3 接口契约（Rust 签名 / struct / enum）

#### core/error.rs

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PvmError {
    #[error("io 错误: {0}")] Io(#[from] std::io::Error),
    #[error("网络/下载错误: {0}")] Http(String),
    #[error("GitHub API 限流(匿名 60/h)，将在 epoch {reset} 重置；可设置 GITHUB_TOKEN 提升到 5000/h")]
    RateLimited { reset: String },
    #[error("解压错误: {0}")] Archive(String),
    #[error("SHA256 校验失败: 期望 {expected}, 实际 {actual}")]
    Checksum { expected: String, actual: String },
    #[error("未找到版本: {0}")] VersionNotFound(String),
    #[error("版本未安装: {0}")] NotInstalled(String),
    #[error("版本来源有歧义，请用 @standalone/@org 或 --source 指定。候选: {0}")]
    Ambiguous(String),
    #[error("配置错误: {0}")] Config(String),
    #[error("未设置 Python 版本（无 PVM_VERSION、无 .python-version、无 global）")]
    NoVersionConfigured,
    #[error("安装器失败({code})，详见日志: {log}")] Installer { code: i32, log: String },
    #[error("Windows 系统调用失败: {0}")] Win(String),
}
pub type Result<T> = std::result::Result<T, PvmError>;
```

> 退出码映射：`Http`/`RateLimited`→4；`Checksum`→5；`NotInstalled`→3；`Installer`→1（其 code 仅写日志/错误文本，不直接当进程退出码）；其余→1。
> 修订：新增 `Installer { code, log }`，承载 python.org 安装器非零退出（见 §7.2），避免用 `Http`/`anyhow` 笼统替代。

#### core/version.rs

```rust
/// 解释器来源。canonical id 后缀与 CLI flag 值不同，见 §8。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source { Standalone, Org }

impl Source {
    /// canonical id 中的后缀（磁盘标识）
    pub fn id_suffix(self) -> &'static str {
        match self { Source::Standalone => "standalone", Source::Org => "org" }
    }
    /// CLI --source 的取值（cpython == python.org 官方）
    pub fn cli_value(self) -> &'static str {
        match self { Source::Standalone => "standalone", Source::Org => "cpython" }
    }
    pub fn from_cli(s: &str) -> Option<Self> {
        match s { "standalone" => Some(Self::Standalone),
                  "cpython" | "org" => Some(Self::Org), _ => None }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PythonVersion {
    pub source: Source,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub freethreaded: bool,   // standalone 的 freethreaded 变体（id 加 't'）
}

impl PythonVersion {
    /// canonical id："cpython-3.12.7-standalone" / "cpython-3.12.7-org" / "cpython-3.13.14t-standalone"
    pub fn canonical(&self) -> String;
    pub fn xyz(&self) -> String;                 // "3.12.7"
    pub fn parse_canonical(id: &str) -> Result<Self>;
}

/// 用户输入 selector，接受 "3.12" / "3.12.7" / "3.12.7@org" / canonical / "latest" / "system"
#[derive(Clone, Debug)]
pub enum VersionSelector {
    Exact { ver: (u32, u32, u32), source: Option<Source>, freethreaded: bool },
    PartialMinor { major: u32, minor: u32, source: Option<Source>, freethreaded: bool },
    PartialMajor { major: u32, source: Option<Source> },
    Latest { source: Option<Source> },
    Canonical(PythonVersion),
    System,
}
pub fn parse_selector(s: &str) -> Result<VersionSelector>;
```

> 修订：`PartialMinor` 增加 `freethreaded` 字段，使 `3.12t`（部分版本 + 自由线程）可表达，与 `Exact.freethreaded`、`.python-version` 中手写 `3.13t` 解析一致（呼应草案未决问题 4）。`PythonVersion` 自定义 `Ord`/`PartialOrd`（先比 `(major,minor,patch)` semver 序，再 `freethreaded` false<true，再 `source`），用于「部分版本取最高 patch」。

#### core/paths.rs

```rust
pub struct Paths { pub root: PathBuf }   // root = %USERPROFILE%\.pvm 或 --root/PVM_ROOT

impl Paths {
    pub fn discover(root_override: Option<PathBuf>) -> Result<Self>;
    pub fn versions(&self) -> PathBuf;
    pub fn shims(&self) -> PathBuf;
    pub fn venvs(&self) -> PathBuf;
    pub fn cache(&self) -> PathBuf;
    pub fn logs(&self) -> PathBuf;
    pub fn backup(&self) -> PathBuf;
    pub fn bin(&self) -> PathBuf;                  // root\bin（shim 模板所在）
    pub fn config_file(&self) -> PathBuf;          // root\config.toml
    pub fn global_version_file(&self) -> PathBuf;  // root\version
    pub fn version_dir(&self, v: &PythonVersion) -> PathBuf;   // root\versions\<canonical>
    /// 关键：按来源选择 python\python.exe(standalone) 或 python.exe(org)
    pub fn python_exe(&self, v: &PythonVersion) -> PathBuf;
    /// pythonw 对应路径（同目录的 pythonw.exe）
    pub fn pythonw_exe(&self, v: &PythonVersion) -> PathBuf;
    pub fn scripts_dir(&self, v: &PythonVersion) -> PathBuf;   // .../Scripts
}
```

> 修订：补 `bin()`（rehash 取 shim 模板）、`pythonw_exe()`（shimw 解析目标）。`discover` 用 `directories::UserDirs`/`BaseDirs` 定位 home。

#### core/config.rs

```rust
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Config {
    pub default_source: Option<String>,  // "standalone" | "cpython"
    pub pip_mirror: Option<String>,      // 别名
    pub disable_auto_switch: bool,
    // 注意：不再存 global，全局版本以 root\version 文件为唯一权威（见 §6 裁决）
}
impl Config {
    pub fn load(paths: &Paths) -> Result<Self>;
    pub fn save(&self, paths: &Paths) -> Result<()>;  // 临时文件 + rename 原子写
    pub fn default_source_resolved(&self) -> Source;  // 缺省 → Standalone
}
```

> 修订（呼应草案未决问题 5）：**删除 `Config.global` 字段**。全局版本只存 `root\version`，消除双写不一致。所有读路径统一走 `resolve_effective`。

#### core/resolve.rs

```rust
pub enum ResolvedFrom {
    Shell,              // PVM_VERSION
    Local(PathBuf),     // 命中的 .python-version 路径
    Global,             // root\version
}
pub struct Effective {
    pub version: PythonVersion,
    pub from: ResolvedFrom,
    pub interpreter_dir: PathBuf,   // version_dir
}

/// shim 与 `pvm current` 共用。顺序：PVM_VERSION > 向上找 .python-version > root\version > Err
pub fn resolve_effective(cwd: &Path, paths: &Paths) -> Result<Effective>;

/// 把 selector 解析为已安装的具体 PythonVersion；多来源未指定时返回 Ambiguous
pub fn resolve_installed(sel: &VersionSelector, default_source: Source, paths: &Paths)
    -> Result<PythonVersion>;

/// 向上查找 .python-version，返回 (文件路径, 原始内容)
pub fn find_dotfile_upwards(start: &Path) -> Result<Option<(PathBuf, String)>>;
```

#### core/download.rs

```rust
pub struct DownloadOpts<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub expect_sha256: Option<&'a str>,  // None 表示 --no-verify 或无摘要
    pub quiet: bool,                      // 抑制 indicatif
}
/// ureq 流式下载到 dest.part，校验后 rename 到 dest
pub fn download_to(opts: &DownloadOpts) -> Result<()>;
pub fn verify_sha256(file: &Path, want: &str) -> Result<()>;
```

> ureq 3.x 读取响应体：`resp.body_mut().as_reader()` 配合 `std::io::copy` 流式写 `dest.part`；进度用 `indicatif::ProgressBar` 包 reader。`Content-Length` 缺失时退化为无总量进度条。

#### core/archive.rs

```rust
pub enum ArchiveKind { TarGz, TarZst, Zip }

/// 按扩展名/参数分派；解压到 dest（调用方保证 dest 为临时目录，外层做原子 rename）
/// 解压时对每个 entry 做路径穿越校验（拒绝 .. 与绝对路径），写文件用 winpath::long_path。
pub fn extract(archive: &Path, dest: &Path, kind: ArchiveKind) -> Result<()>;
```

> 修订：明确 zip-slip/tar 穿越防护（`zip 8.x` 的 `ZipFile::enclosed_name()`、tar 用 `Entry::path()` 校验），并对长路径加 `\\?\`。`TarZst` 仅在 `zstd-full` feature 下编译可用。

#### core/source_pbs.rs（python-build-standalone）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbsFlavor {
    InstallOnly,            // 默认 .tar.gz（纯 Rust 解压）
    InstallOnlyStripped,    // install_only_stripped.tar.gz
    PgoFull,               // pgo-full .tar.zst（需 zstd-full feature）
}
impl PbsFlavor {
    /// 非 freethreaded 时的资产名后缀（已核实实际命名）
    pub fn suffix(self) -> &'static str;   // "install_only.tar.gz" / "install_only_stripped.tar.gz" / "pgo-full.tar.zst"
    pub fn is_zstd(self) -> bool;          // 仅 PgoFull 为 true
}

#[derive(Debug, Clone)]
pub struct PbsAsset {
    pub python_version: semver::Version,
    pub release_date: String,    // release tag "YYYYMMDD"
    pub freethreaded: bool,
    pub flavor: PbsFlavor,
    pub download_url: String,    // browser_download_url（含 %2B）
    pub size: u64,
    pub sha256: Option<String>,  // API digest "sha256:..." 去前缀
    pub file_name: String,
}

pub const PBS_REPO: &str = "astral-sh/python-build-standalone";
pub const TARGET_TRIPLE: &str = "x86_64-pc-windows-msvc";

/// 枚举（分页 + 限流 + ETag 缓存）。token 取 GITHUB_TOKEN。
/// freethreaded 参数控制是否仅保留自由线程变体。
pub fn list_pbs_assets(token: Option<&str>, flavor: PbsFlavor, freethreaded: bool,
                       paths: &Paths, refresh: bool) -> Result<Vec<PbsAsset>>;

/// 下载 → 解压到临时目录 → 按 flavor 校验解释器子路径 → 原子 rename
pub fn install_pbs(asset: &PbsAsset, v: &PythonVersion, paths: &Paths, no_verify: bool)
    -> Result<()>;
```

> **修订要点（已抽查核实）：**
> - 资产名实测形如 `cpython-3.10.20+20260610-x86_64-pc-windows-msvc-install_only.tar.gz`、`...-install_only_stripped.tar.gz`、`...-pgo-full.tar.zst`。`PbsFlavor::Full` 重命名为 `PgoFull`，`suffix()` 返回 `pgo-full.tar.zst`（草案的裸 `full` 错误，会拼不出真实 URL）。
> - `digest` 字段确为 `"sha256:<hex>"`，去前缀后即 SHA256，**无需再单独下载 SHA256SUMS**。故草案的 `parse_sha256sums` 函数**删除**（GitHub Releases 资产 JSON 自带 digest；若某些旧 release 无 digest，再回退 `--no-verify` 或 HEAD 校验）。
> - freethreaded Windows 资产命名为复合形式（实测近似 `cpython-3.13.x+<date>-x86_64-pc-windows-msvc-freethreaded+pgo-install_only.tar.gz` / `freethreaded+lto-...`），**不是**草案设想的扁平 `freethreaded-install_only`。
>   > 待定: freethreaded 资产的精确后缀（`freethreaded+pgo-` 还是 `freethreaded+pgo+lto-`、是否随版本变化）需在实现时以正则从实际 release JSON 提取，**禁止硬编码后缀字符串**；建议匹配 `freethreaded` 子串 + `install_only.tar.gz` 结尾来选取，并按可用性优先 install_only。
> - 枚举正则（freethreaded 与否两套）：`^cpython-(?P<ver>\d+\.\d+\.\d+)\+(?P<date>\d{8})-x86_64-pc-windows-msvc-(?P<rest>.+)\.tar\.(gz|zst)$`，再按 `rest` 是否含 `freethreaded`、是否以 `install_only`/`install_only_stripped`/`pgo-full` 收尾归类。
> - 安装校验：install_only/stripped 校验 `tmp\python\python.exe`；PgoFull 校验 `tmp\python\install\python.exe`。

#### core/source_pyorg.rs（python.org）

```rust
pub enum PyOrgFlavor { Installer, Embed }  // Embed 为受限备选，不可作 venv 基底

pub struct PyOrgRelease {
    pub version: semver::Version,
    pub is_prerelease: bool,
    pub installer_url: String,    // 始终按 ftp 模板自拼
    pub embed_url: String,
    pub sha256: Option<String>,   // 来自 index-windows.json hash（可能缺）
}

/// 三级回退枚举：index-windows.json → api/v2/downloads → ftp autoindex
pub fn list_remote(paths: &Paths, refresh: bool) -> Result<Vec<PyOrgRelease>>;

/// 官方安装器静默安装（主方案）：缓存 exe → /quiet 装到版本目录
pub fn install_via_installer(v: &PythonVersion, paths: &Paths) -> Result<()>;

/// 卸载：复用缓存原 exe（或重下）→ /quiet /uninstall → 删空目录
pub fn uninstall_via_installer(v: &PythonVersion, paths: &Paths) -> Result<()>;

/// 嵌入式安装（受限备选）：解压 zip → 改 ._pth → get-pip.py
pub fn install_via_embed(v: &PythonVersion, paths: &Paths) -> Result<()>;

/// ftp 模板 URL 拼接
pub fn installer_url(v: &PythonVersion) -> String;  // .../ftp/python/<X.Y.Z>/python-<X.Y.Z>-amd64.exe
```

> 已核实：`https://www.python.org/ftp/python/3.12.7/` 下确有 `python-3.12.7-amd64.exe`，模板成立。

#### core/shim.rs

```rust
/// rehash：为 python/pythonw/pip/pip3 + venv 入口拷贝 shim 模板到 shims\<name>.exe，
/// 并清理 shims\ 中指向已删版本的残留项。
/// 入参 p 提供 root；shim 模板取自 p.bin()（pvm-shim.exe / pvm-shimw.exe）。
pub fn rehash(p: &Paths) -> Result<()>;
```

> 修订：草案签名 `rehash(paths: &Path, p: &Paths)` 双传参冗余且类型混乱，统一为 `rehash(p: &Paths)`。console_scripts 全量 rehash 见 §7.1 决策（默认仅核心 + venv 入口）。

#### core/venv.rs

```rust
#[derive(serde::Serialize, serde::Deserialize)]
pub struct VenvMeta {
    pub name: String,
    pub python_version: String,   // canonical id
    pub source: String,
    pub base_prefix: PathBuf,
    pub created_at: String,       // RFC3339
}

pub struct VenvCreateOpts<'a> {
    pub name: &'a str,
    pub py_selector: Option<&'a str>,
    pub in_project: bool,
    pub path: Option<&'a Path>,
    pub clear: bool,
    pub without_pip: bool,
    pub system_site_packages: bool,
    pub mirror: Option<&'a str>,
}
pub fn venv_create(opts: &VenvCreateOpts, paths: &Paths) -> Result<()>;
pub fn venv_list(paths: &Paths) -> Result<Vec<VenvMeta>>;
pub fn venv_remove(name: &str, paths: &Paths, yes: bool) -> Result<()>;
```

#### core/pip.rs

```rust
pub struct Mirror {
    pub aliases: &'static [&'static str],
    pub display: &'static str,
    pub index_url: &'static str,
    pub trusted_host: Option<&'static str>,
}
pub const MIRRORS: &[Mirror] = /* 见 §7.4 */;

#[derive(Clone, Copy)]
pub enum Scope { Global, Venv }

pub fn pip_ini_path(scope: Scope, venv: Option<&Path>, paths: &Paths) -> Result<PathBuf>;
pub fn lookup_mirror(key: &str) -> Option<&'static Mirror>;
pub fn pip_mirror_set(name_or_url: &str, scope: Scope, venv: Option<&Path>,
                      no_trusted: bool, paths: &Paths) -> Result<()>;
pub fn pip_mirror_reset(scope: Scope, venv: Option<&Path>, paths: &Paths) -> Result<()>;
pub fn pip_mirror_show(scope: Scope, venv: Option<&Path>, paths: &Paths) -> Result<()>;
```

#### core/winpath.rs

```rust
/// 加 \\?\ 前缀的绝对路径（用于文件操作避免 MAX_PATH）。已是 \\?\ 前缀则原样返回。
pub fn long_path(p: &Path) -> PathBuf;

/// HKCU\Environment 的 Path 前插 shims 目录（保留 REG_EXPAND_SZ 类型），写后广播
pub fn prepend_shims_to_user_path(shims_dir: &str, paths: &Paths) -> Result<()>;
pub fn remove_shims_from_user_path(shims_dir: &str) -> Result<()>;
fn broadcast_setting_change();  // SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, ...)

/// shim 专用：安装返回 TRUE 的空 Ctrl+C handler，让子进程接管信号
pub unsafe fn install_noop_ctrl_handler();
```

#### cli.rs（clap derive 草图）

```rust
#[derive(clap::Parser)]
#[command(name = "pvm", version, about = "Windows Python 版本管理器")]
pub struct Cli {
    #[arg(long, global = true, env = "PVM_ROOT")] pub root: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value_t = ColorWhen::Auto)] pub color: ColorWhen,
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)] pub verbose: u8,
    #[arg(short = 'q', long, global = true)] pub quiet: bool,
    #[arg(long, global = true)] pub json: bool,
    #[arg(short = 'y', long, global = true)] pub yes: bool,
    #[command(subcommand)] pub command: Command,
}

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum SourceArg { Standalone, Cpython }  // Cpython => id 后缀 "-org"

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum FlavorArg { InstallOnly, Stripped, Full }  // Full => PbsFlavor::PgoFull

#[derive(clap::Subcommand)]
pub enum Command {
    #[command(alias = "i")]
    Install { versions: Vec<String>,
        #[arg(long, value_enum)] source: Option<SourceArg>,
        #[arg(long, value_enum, default_value_t = FlavorArg::InstallOnly)] flavor: FlavorArg,
        #[arg(long)] freethreaded: bool,
        #[arg(long)] force: bool,
        #[arg(long)] skip_existing: bool,
        #[arg(short = 'g', long = "set-global")] set_global: bool,
        #[arg(long)] no_verify: bool,
        #[arg(long)] mirror: Option<String> },
    #[command(alias = "rm", alias = "remove")]
    Uninstall { versions: Vec<String>,
        #[arg(short = 'y', long)] yes: bool,
        #[arg(long)] keep_venvs: bool },
    #[command(alias = "ls")]
    List { #[arg(long)] remote: bool, #[arg(long, value_enum)] source: Option<SourceArg>,
        #[arg(long)] bare: bool, #[arg(long)] all: bool },
    #[command(name = "ls-remote")]
    LsRemote { #[arg(long, value_enum)] source: Option<SourceArg>,
        #[arg(long)] all: bool, #[arg(long)] refresh: bool },
    #[command(alias = "use")] Global { version: Option<String> },
    Local { version: Option<String>, #[arg(long)] unset: bool },
    Shell { version: Option<String>, #[arg(long)] unset: bool },
    Which { exe: Option<String> },
    #[command(alias = "version")] Current,
    #[command(alias = "run")]
    Exec { #[arg(long)] version: Option<String>, #[arg(last = true)] cmd: Vec<String> },
    Venv { #[command(subcommand)] cmd: VenvCmd },
    #[command(name = "pip-mirror", alias = "mirror")]
    PipMirror { #[command(subcommand)] cmd: MirrorCmd },
    Init { #[arg(long)] path_only: bool, #[arg(long, value_enum)] shell: Option<ShellKind> },
    Rehash,
    #[command(name = "self")] SelfCmd { #[command(subcommand)] cmd: SelfCmd },
    Doctor,
    Root,
}
```

> 修订：
> - 新增 `FlavorArg`（与表格三值一致），并在 `install` 上以 `default_value_t` 给默认；草案表头有 `--flavor` 但 enum 漏定义。
> - `Uninstall` 显式声明 `-y/--yes`（草案命令表写了 `-y` 但 enum 漏；虽有全局 `--yes`，保留局部以兼容 `uninstall <v> -y` 的直觉，二者择一即可——**裁决：删局部、统一用全局 `--yes`**，此处保留注释提示，避免 clap 重复定义报错）。
>   > 待定: `-y` 走全局还是局部，二选一，不可两处都定义同名短 flag。建议仅全局。

---

## 5. Cargo.toml 依赖清单

> 版本号采用「兼容范围（caret）」写法；右侧括注核实到的当前版本。HTTP 库默认 `ureq`（PBS/pyorg 枚举与下载为低并发，够用）。

```toml
[package]
name = "pvm"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"

[lib]
name = "pvm"
path = "src/lib.rs"

[[bin]]
name = "pvm"
path = "src/main.rs"

[[bin]]
name = "pvm-shim"
path = "src/bin/shim.rs"

[[bin]]
name = "pvm-shimw"
path = "src/bin/shimw.rs"

[dependencies]
# --- CLI ---
clap            = { version = "4.6",  features = ["derive", "env"] }   # 核实 4.6.x
clap_complete   = "4.6"
anstream        = "0.6"
anstyle         = "1.0"
dialoguer       = "0.11"
indicatif       = "0.18"                                              # 核实 0.18.x

# --- HTTP（默认 ureq，纯阻塞 + rustls，无 tokio）---
ureq            = { version = "3.3", features = ["json"] }            # 核实 3.3.x；默认含 rustls

# --- 序列化 / 配置 ---
serde           = { version = "1",     features = ["derive"] }
serde_json      = "1"
toml            = "0.8"
rust-ini        = { package = "ini", version = "0.21" }              # 写 pip.ini，保序合并

# --- 解压 ---
tar             = "0.4"
flate2          = "1.1"                                              # 纯 Rust(miniz_oxide)
zip             = "8"                                                # 嵌入式 zip
zstd            = { version = "0.13", optional = true }              # 仅 pgo-full 包；feature 控制

# --- 校验 / 编码 ---
sha2            = "0.11"
hex             = "0.4"

# --- 版本号 ---
semver          = "1"
regex           = "1"

# --- 路径 / 时间 ---
directories     = "6"                                               # 定位 home/AppData
dunce           = "1"                                               # 规范化长路径前缀
chrono          = { version = "0.4", default-features = false, features = ["clock", "serde"] }
url             = "2"

# --- 错误 ---
anyhow          = "1"
thiserror       = "2"

# --- 自更新 ---
self_update     = { version = "0.42", default-features = false, features = ["rustls"] }

# --- Windows API ---
winreg          = "0.55"                                            # HKCU\Environment 读写（get/set_raw_value 保类型）
windows-sys     = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_System_Console",            # SetConsoleCtrlHandler / GenerateConsoleCtrlEvent
    "Win32_UI_WindowsAndMessaging",    # SendMessageTimeoutW / WM_SETTINGCHANGE
    "Win32_System_JobObjects",         # CreateJobObjectW 等（兜底孤儿回收）
    "Win32_System_Threading",
] }

[features]
default = []
zstd-full = ["dep:zstd"]   # 启用后才能装 --flavor full（pgo-full.tar.zst），引入 cc/C 工具链

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

`.cargo/config.toml`：

```toml
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

> **依赖裁决**：
> - HTTP 走 `ureq`，**移除 reqwest 备选注释**（避免误导实现者，PBS 接口已统一为 ureq 语义）。
> - 注册表走 `winreg`（保 `REG_EXPAND_SZ`）+ `windows-sys`（控制台/消息/Job）。不引入 `windows`(0.62) 高层 crate。
> - 已核实 `windows-sys 0.59` 的 `Win32_System_Console`、`Win32_UI_WindowsAndMessaging`、`Win32_System_JobObjects` 三个 feature 名有效。
> - `directories` 与 `dirs` 二者择一，选 `directories`，不再引入 `dirs`。`junction` 不引入。
> - **shim 二进制的依赖瘦身**：`pvm-shim`/`pvm-shimw` 理论上只需 `winreg`/`windows-sys`/`directories`/`dunce`/`thiserror`，不需 ureq/zip/zstd/tar/flate2/self_update/indicatif/clap。Cargo 单 crate 无法对不同 bin 做依赖裁剪（共享 `[dependencies]`），但因 `lto + strip + 死代码消除`，shim 实际不会链接未调用的网络/解压符号，体积可接受。
>   > 待定: 若 shim 体积仍偏大，改为 workspace 多 crate（`pvm-core` lib + `pvm-cli` bin + `pvm-shim` bin），shim crate 只依赖 `pvm-core` 的精简子集。本期先单 crate，体积超标再拆。

---

## 6. 核心数据结构

集中定义五类共享类型（已在 §4.3 给出完整签名，此处汇总语义与字段不变量）：

| 类型 | 定义位置 | 关键不变量 |
|---|---|---|
| `PythonVersion` | core/version.rs | `canonical()` 唯一标识磁盘目录；`source` + `(major,minor,patch)` + `freethreaded` 共同决定身份；同 xyz 不同 source 可并存；实现 `Ord` 用于取最高 patch |
| `Source` | core/version.rs | 二值枚举；`id_suffix()`(磁盘=`standalone`/`org`) 与 `cli_value()`(CLI=`standalone`/`cpython`) 必须区分，是全局唯一命名不一致点 |
| `Config` | core/config.rs | `default_source` 缺省视为 `standalone`；**不存 global**，全局版本以 `root\version` 为唯一权威 |
| `Paths` | core/paths.rs | 所有路径从 `root` 派生；`python_exe()` 按 `source` 返回 `python\python.exe`(standalone) 或 `python.exe`(org) |
| `PvmError` | core/error.rs | 与退出码一一映射；`Ambiguous`/`NotInstalled`/`NoVersionConfigured`/`RateLimited`/`Checksum`/`Installer` 为必须显式区分的语义错误，禁止用 `anyhow::anyhow!` 笼统替代 |

辅助结构：`VersionSelector`（用户输入）、`Effective`/`ResolvedFrom`（生效解析）、`PbsAsset`/`PbsFlavor`、`PyOrgRelease`/`PyOrgFlavor`、`VenvMeta`、`Mirror`/`Scope`。

> **裁决（呼应草案未决问题 5）**：全局版本单一来源化——只读写 `root\version`，`config.toml` 不再镜像 `global`，彻底消除一致性风险。

---

## 7. 四类功能的实现要点与关键算法伪代码

### 7.1 版本切换与 shim

**版本解析优先级（resolve.rs，shim 与 CLI 共用）**

```
fn resolve_effective(cwd, paths):
    if env "PVM_VERSION" 非空:
        return finalize(trim(PVM_VERSION), Shell, paths)
    if (file, raw) = find_dotfile_upwards(cwd):   # 逐级 parent，直到盘符根
        return finalize(trim(raw), Local(file), paths)
    if raw = read(paths.global_version_file()):
        return finalize(trim(raw), Global, paths)
    Err(NoVersionConfigured)   # 不静默回退 system python

fn finalize(req, from, paths):
    sel = parse_selector(req)                 # 容忍部分版本 "3.12"
    v   = resolve_installed(sel, config.default_source_resolved(), paths)
          # 部分版本→已装最高 patch；多来源未指定→Ambiguous
    interpreter_dir = paths.version_dir(v)
    return Effective{v, from, interpreter_dir}
```

读 `.python-version`/`version` 时统一 `trim` 空白、BOM、CRLF。

**shim 启动（winpath.rs + bin/shim.rs）**

```
fn run_shim() -> !:
    stem  = lowercase(current_exe().file_stem())     # python / pythonw / pip / pip3 / <entrypoint>
    paths = Paths::discover(env PVM_ROOT)
    eff   = resolve_effective(current_dir(), paths)
            else { eprintln!("pvm: {e}"); exit(127) }
    target = locate_real_exe(eff, stem)
             # python  -> paths.python_exe(eff.version)
             # pythonw -> paths.pythonw_exe(eff.version)
             # 其它    -> paths.scripts_dir(eff.version)\<stem>.exe
             else { eprintln!("pvm: 入口 {stem} 不存在于 {ver}"); exit(127) }
    install_noop_ctrl_handler()                       # 返回 TRUE 的空 handler
    status = Command::new(long_path(target))
                .args(args_os().skip(1))              # 原样透传，由标准库做 Windows 转义
                .status()                             # 继承 stdio，不加 CREATE_NEW_PROCESS_GROUP
             else exit(126)
    # 可选：Job Object + JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE 兜底孤儿回收（失败降级，不致命）
    exit(status.code().unwrap_or(1))                  # 透传退出码
```

要点：
- pythonw 用 `pvm-shimw.exe`（GUI 子系统）避免闪控制台。
- Ctrl+C 依赖「同控制台广播」：shim 装空 handler 不被默认 handler 杀，信号自然送达子进程。不主动 `GenerateConsoleCtrlEvent`。
- shim 身份用 `current_exe()` 文件名判断，**不用** `args[0]`（Windows 上不可靠）。
- Windows install_only **只有 `python.exe`，无 `python3.exe`**；shim 不暴露 `python3` 入口（pip3 仍暴露，因 Scripts 下有 `pip3.exe`）。

**PATH 注入（winpath.rs）**

```
fn prepend_shims_to_user_path(shims_dir, paths):
    备份原始 Path 到 paths.backup()\user-path.bak（仅首次）
    env = HKCU\Environment, open(KEY_READ|KEY_WRITE)
    (old, vtype) = env.get_raw_value("Path")          # 保留 REG_EXPAND_SZ 原文，勿无脑转换
    parts = old.split(';').filter(非空)
    parts.retain(p != shims_dir, ignore_ascii_case)   # 去重，移除旧位置
    new = join(';', [shims_dir] ++ parts)
    env.set_raw_value("Path", RegValue{bytes: wide(new), vtype: vtype})  # 原类型回写
    SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, "Environment",
                        SMTO_ABORTIFHUNG, 5000, &out)   # 仅影响新开进程，需提示重开终端
```

> 修订：回写时复用读到的 `vtype`（若原为 `REG_EXPAND_SZ` 则保持，若原为 `REG_SZ` 则保持），而非强制写 `REG_EXPAND_SZ`，避免改变用户原有语义。

**rehash**：扫所有已装版本与 venv 的 `Scripts\*.exe` 入口 → 在 `shims\` 拷贝 `pvm-shim.exe`/`pvm-shimw.exe` 为对应名 → 清理指向已删版本的残留 shim。install/uninstall/venv create 后自动调用。

> **决策（呼应草案未决问题 2）**：默认仅 rehash 核心入口（`python`/`pythonw`/`pip`/`pip3`）+ 当前 global/local 生效版本及各 venv 的 `Scripts\` 全量 entry-point exe（如 black/pytest）。console_scripts 全量 rehash 对所有已装版本默认关闭，可由 `config.toml` 的 `rehash_all_scripts = true` 开启。理由：贴近 pyenv 体验但默认控制 shim 数量与清理复杂度。

### 7.2 双源下载安装

**安装总编排（install.rs）**

```
fn install(selector, source_arg, flavor_arg, freethreaded, flags, paths):
    src = source_arg ?? config.default_source_resolved()   # 缺省 Standalone
    if src == Org && (freethreaded || flavor_arg != InstallOnly):
        Err("freethreaded/flavor 仅 standalone 支持")
    v = 预解析目标版本(selector, src, freethreaded)        # 远端枚举定位具体 xyz
    if skip_existing && 已装(v): return Ok
    if force && 已装(v): uninstall(v)
    match src:
      Standalone -> install_pbs_flow(v, flavor_arg, freethreaded, flags, paths)
      Org        -> source_pyorg::install_via_installer(v, paths)
    if set_global: write(paths.global_version_file(), v.canonical())   # 原子写
    shim::rehash(paths)

# CLI 层校验：flavor==Full 但未启用 zstd-full feature → 立即报清晰错误
#   "本二进制未启用 zstd-full，无法安装 --flavor full，请用预编译带 zstd-full 的版本或改用默认 install_only"
```

> 修订（呼应草案未决问题 6）：`--flavor full` 在未编译 `zstd-full` 时于 CLI 层即拒绝，不留到运行时解压失败。

**PBS（source_pbs.rs）枚举与安装**

```
fn list_pbs_assets(token, flavor, freethreaded, paths, refresh):
    若缓存 pbs-index.json 存在且 !refresh:
        带 If-None-Match: <etag> 请求；304 → 用缓存
    url = ".../releases?per_page=100&page=1"
    loop:
        req 加头 User-Agent:pvm(强制), Accept:application/vnd.github+json,
                X-GitHub-Api-Version:2022-11-28, [Bearer token]
        resp = send
        check_rate_limit(resp)        # x-ratelimit-remaining==0 → Err(RateLimited{reset})
        for rel in resp.json(): for a in rel.assets:
            if 正则匹配(a.name, triple, flavor, freethreaded):
                push PbsAsset{ ver, date, freethreaded, flavor, url(含%2B), size,
                               sha256 = a.digest 去 "sha256:" 前缀, file_name }
        next = parse_link_next(resp.headers)   # Link: rel="next"
        if next.none: break else url = next
    sort_by (ver desc, date desc); dedup_by (python_version, freethreaded)  # 同 xyz 多日期保最新
    写缓存 + ETag

fn install_pbs(asset, v, paths, no_verify):
    file = cache\<file_name>
    download_to(asset.url, file, asset.sha256 unless no_verify)   # 流式 + SHA256
    tmp  = version_dir.parent\.tmp-<canonical>-<rand>            # 同盘临时目录，保证 rename 原子
    extract(file, tmp, if asset.flavor.is_zstd() {TarZst} else {TarGz})
    interp = if asset.flavor==PgoFull { tmp\python\install\python.exe }
             else { tmp\python\python.exe }
    assert exists(interp)
    若 PgoFull: 将 tmp\python\install 规整到 version_dir 布局（使 python_exe 落在 python\python.exe）
        > 待定: full 包的目录规整策略（直接 rename install 层 还是保留 install 子层）需与
        > Paths::python_exe 的 standalone 分支保持一致；建议把 install\* 提升一层以复用同一路径函数。
    rename(tmp -> version_dir)             # 原子；失败清 tmp
```

资产名模式（**已核实**）：`cpython-<X.Y.Z>+<YYYYMMDD>-x86_64-pc-windows-msvc-install_only.tar.gz`、`...-install_only_stripped.tar.gz`、`...-pgo-full.tar.zst`，URL 中 `+`→`%2B`。`digest` 字段为 `"sha256:<hex>"`。默认 `install_only`（gzip，纯 Rust）；`--flavor full` 走 `pgo-full.tar.zst`（需 `zstd-full` feature）。

**python.org（source_pyorg.rs）安装**

```
fn install_via_installer(v, paths):
    exe = download_to_cache(installer_url(v), sha256_if_known)   # 缓存以备卸载！
    target = version_dir(v)                                      # ...\cpython-<ver>-org
    status = Command::new(exe).args([
        "/quiet", "InstallAllUsers=0",
        "TargetDir="+to_windows_path(target),                    # 反斜杠路径，放 home 下规避 UAC
        "Include_launcher=0","Shortcuts=0","AssociateFiles=0",
        "PrependPath=0","AppendPath=0","CompileAll=0",
        "Include_test=0","Include_doc=0","Include_pip=1","Include_tcltk=1",
    ]).arg("/log").arg(logs\install-<ver>.log).status()
    match status.code:
        Some(0) | Some(3010) -> Ok                               # 3010=成功需重启
        Some(1602) -> Err(Installer{1602,log})  # 用户取消
        Some(1638) -> Err(Installer{1638,log})  # 已装同版本/降级阻止
        Some(1603) | _       -> Err(Installer{code,log})         # 致命错误，见 log

fn uninstall_via_installer(v, paths):
    exe = cached_or_redownload(installer_url(v))       # 缓存丢失则按版本名重下
    Command::new(exe).args(["/quiet","/uninstall"]).status()  # 必须用原 bootstrapper
    remove_dir_all(version_dir(v))
```

> **已核实/修订要点：**
> - 官方文档确认安装器支持 `/quiet`、`/uninstall`、`/log`、`InstallAllUsers`(默认 0)、`TargetDir`、`PrependPath`(默认 0)、`AppendPath`、`Include_launcher`(默认 1)、`Include_test`、`Include_pip`、`Include_tcltk`、`Include_doc`、`Shortcuts`(默认 1)、`AssociateFiles`、`CompileAll`(默认 0) 等。草案参数集全部合法。
> - 退出码 `0`/`3010` 判成功，`1602`(取消)/`1638`(已存在)/`1603`(致命) 为标准 MSI 约定。**强制 `InstallAllUsers=0`**（per-user，不暴露给用户，避免 UAC）。
> - **`Include_launcher=0` 行为提醒**：3.13+ 安装器的「launcher」已是新版 Python Install Manager；`Include_launcher=0` 跳过它（pvm 自管多版本，不需要系统级 `py.exe`），合理。但这也意味着不安装 `py.exe`，文档需说明 pvm 用户应用 `pvm` 自身做版本分发而非 `py`。
>   > 待定: 极少数 3.13+ per-user + 自定义 `TargetDir` 组合仍可能弹 UAC（cpython 已知问题）；须捕获非 0/3010 退出码并提示「可改用 `--source standalone` 免 UAC」。
> - `TargetDir` 必须传 Windows 反斜杠绝对路径（不可带 `\\?\` 前缀，安装器不识别）。用户名含非 ASCII/空格时建议回退 standalone。

**python.org 枚举三级回退**：`index-windows.json`（筛 `company==PythonCore && tag.ends_with("-64")`，带 hash）→ `api/v2/downloads/release/?is_published=true`（筛 `!pre_release`）→ ftp autoindex（正则 `^\d+\.\d+\.\d+/$`）。下载 exe URL **始终按 ftp 模板自拼**（`installer_url`），不假定 manifest 的 url 即 amd64.exe；缓存到 `pyorg-index.json` 带 TTL。

> 已核实 ftp 模板：`https://www.python.org/ftp/python/<X.Y.Z>/python-<X.Y.Z>-amd64.exe` 真实存在。

### 7.3 venv

```
fn venv_create(opts, paths):
    v = resolve_installed(parse_selector(opts.py_selector ?? 当前生效版本), default_source, paths)
    if v.source==Org && is_embed(v): Err("嵌入式版本不可作 venv 基底")
    py = paths.python_exe(v)
    if !exists(py): Err("Python {v} 未安装，请先 pvm install")
    target = opts.in_project ? cwd\.venv
           : opts.path ? opts.path
           : paths.venvs()\<name>
    if exists(target) && !clear && nonempty(target): Err("目标已存在，用 --clear")
    cmd = Command::new(py).arg("-m").arg("venv")
    if clear: cmd.arg("--clear");  if without_pip: cmd.arg("--without-pip")
    if system_site_packages: cmd.arg("--system-site-packages")
    cmd.arg(target)
    status = cmd.status()                    # stderr 直通，不吞
    if !success: Err("python -m venv 失败 {code}")
    if !in_project:                          # 仅集中式写元数据
        write_json(target\.pvm-venv.json, VenvMeta{...})
    if opts.mirror: pip_mirror_set(mirror, Scope::Venv, Some(target), false, paths)
    print_activation_hint(target, v.canonical())
    shim::rehash(paths)                      # 暴露 venv 入口

fn print_activation_hint(venv, ver):
    s = venv\Scripts
    打印三种 shell:
      PowerShell : & '<s>\Activate.ps1'
      cmd.exe    : <s>\activate.bat
      Git Bash   : source '<s>/activate'    # 反斜杠转正斜杠
      (PS 执行策略提示: Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass)
```

要点：venv 目录在 Windows 固定 `Scripts\`（非 `bin/`）。pvm 不注入父 shell 环境，只打印激活命令。`--in-project` 的 `.venv` 不写元数据、不被 `venv list` 列出（文档明示）。standalone install_only 与 org 安装器均自带 ensurepip/venv，统一走 `python -m venv` 无分支；唯嵌入式（embed）版本禁止作基底。

### 7.4 pip 镜像

**内置镜像表（已核实，2026-06）**

```rust
pub const MIRRORS: &[Mirror] = &[
  Mirror{aliases:&["tuna","tsinghua"], display:"清华 TUNA",
     index_url:"https://pypi.tuna.tsinghua.edu.cn/simple",
     trusted_host:Some("pypi.tuna.tsinghua.edu.cn")},
  Mirror{aliases:&["aliyun","ali"], display:"阿里云",
     index_url:"https://mirrors.aliyun.com/pypi/simple/",
     trusted_host:Some("mirrors.aliyun.com")},
  Mirror{aliases:&["ustc"], display:"中科大",
     index_url:"https://mirrors.ustc.edu.cn/pypi/simple",
     trusted_host:Some("mirrors.ustc.edu.cn")},
  Mirror{aliases:&["tencent","qcloud"], display:"腾讯云",
     index_url:"https://mirrors.cloud.tencent.com/pypi/simple",
     trusted_host:Some("mirrors.cloud.tencent.com")},
  Mirror{aliases:&["huawei","hwcloud"], display:"华为云",
     index_url:"https://repo.huaweicloud.com/repository/pypi/simple",
     trusted_host:Some("repo.huaweicloud.com")},
  Mirror{aliases:&["pypi","official"], display:"官方源",
     index_url:"https://pypi.org/simple", trusted_host:None},
];
```

> **镜像表冲突裁决**：以「venv+pip 子系统」设计为权威（5 个国内镜像 + 官方），采用清华短域名 `pypi.tuna`。CLI 子设计中的 `douban`（已长期不可用）不纳入。

**写入算法（pip.rs，保序合并不破坏用户配置）**

```
fn pip_mirror_set(name_or_url, scope, venv, no_trusted, paths):
    (index_url, trusted) = lookup_mirror(name_or_url)
        ?? { validate http(s); host = Url::parse(url).host_str(); (url, Some(host)) }
    path = scope==Global ? %APPDATA%\pip\pip.ini : <venv>\pip.ini
    mkdir path.parent
    if exists(path): backup_once(path -> pip.ini.pvm.bak)   # 仅一次
    conf = exists(path) ? Ini::load(path) : Ini::new()       # rust-ini 保序
    conf["global"]["index-url"] = index_url
    if !no_trusted && trusted: conf["global"]["trusted-host"] = trusted
    else: conf["global"].delete("trusted-host")
    conf.write(path)

fn pip_mirror_reset(scope, venv, paths):     # 仅删 pvm 写入项，不删整文件
    conf["global"].remove("index-url"); conf["global"].remove("trusted-host")
    conf.write(path)
```

要点：默认写 `pip.ini`（持久、被解释器继承、与 pyenv/uv 习惯一致），`--env` 仅打印临时 `set/$env:` 命令。`https` 源默认写 `trusted-host`（兼容公司 MITM 代理），`--no-trusted-host` 可关。全局位置 `%APPDATA%\pip\pip.ini`，venv 级 `<venv>\pip.ini`。

---

## 8. 双来源版本 id 命名统一方案与默认来源

这是 6 个子设计中**最大的冲突点**，本 SPEC 统一裁决如下。

### 8.1 canonical id（磁盘与内部唯一标识）

```
cpython-<major.minor.patch>[t]-<source_suffix>
```

- `source_suffix` ∈ `{ standalone, org }`。
- `t` 后缀仅 standalone freethreaded 变体使用（如 `cpython-3.13.14t-standalone`）。
- 例：`cpython-3.12.7-standalone`、`cpython-3.12.7-org`、`cpython-3.13.14-standalone`。
- 同一 xyz 双来源可并存于不同目录，互不覆盖。
- 前缀 `cpython-` 固定，为 `pypy-`/`graalpy-` 预留命名空间。

> **统一裁决**：工程架构子设计曾用裸 `standalone-3.12.7` / `cpython-3.12.7` 作目录名，与 CLI 子设计的 `cpython-3.12.7-standalone` / `-org` 冲突。**本 SPEC 一律采用后者**。`canonical()` 与 `parse_canonical()` 必须对 `t` 后缀做双向无损：序列化时 `freethreaded → patch 后加 t`，解析时识别 `<patch>t`。

### 8.2 CLI 输入语法（version selector）

解析优先级：

1. 完整 canonical：`cpython-3.12.7-standalone`。
2. 后缀消歧：`3.12.7@standalone` / `3.12.7@org`（`@` 为来源分隔符）。
3. 纯版本号 + `--source`：`pvm install 3.12.7 --source cpython`。
4. 部分版本：`3.12`→该 minor 线最新已装/可装 patch；`3`→该 major 线最新；`latest`→全局最新稳定。
5. freethreaded：`3.13t` / `3.13.14t` 或 `--freethreaded`（仅 standalone，3.13+）。
6. 别名：`system`（PATH 上 pvm 之外的 python，保留语义；本期 `which`/`exec` 可解析，安装类命令拒绝）。

**歧义裁决规则**：
- `global`/`local`/`uninstall`/`venv --version`/`exec --version`：若本地同 xyz 装了两来源且未带 `@source`/`--source`，**报 `Ambiguous` 并列候选，禁止静默挑选**。
- `install`：未指定来源时按默认来源策略静默选，无歧义。
- `local`/`global` 写盘时落**已解析的 canonical id**（非浮动 selector），保证可复现。
- freethreaded 仅 standalone 合法；`--source cpython --freethreaded` 或 `3.13t@org` 报参数错误（退出码 2）。

### 8.3 `--source` flag 值与 id 后缀的映射（唯一命名不一致点）

| CLI `--source` 值 | 含义 | canonical id 后缀 |
|---|---|---|
| `standalone` | python-build-standalone | `-standalone` |
| `cpython` | python.org 官方安装器 | `-org` |

`cpython` 表示「官方 CPython 安装器」（直觉对齐），磁盘后缀用 `org`。`Source::cli_value()` 与 `Source::id_suffix()` 分别承载二者，`Source::from_cli` 同时接受 `cpython` 与 `org`。此映射必须写入 `--help` 文本与文档。

### 8.4 默认来源策略

**默认 `standalone`。** 理由：解压即用、内含 pip、可安装到任意目录、无需管理员、无注册表写入、天然支持同版本号多份并存，最契合版本管理器语义，且默认走 `.tar.gz` 纯 Rust 解压零外部依赖。`--source cpython`（python.org）保留给需官方签名构建、官方 launcher 或特定 C-ext 兼容性的用户。可由 `config.toml` 的 `default_source` 或 `PVM_DEFAULT_SOURCE` 覆盖。

---

## 9. 风险与未决问题

### 9.1 已识别风险（按子系统）

**版本切换 / shim**
- shim 每次调用多一层进程启动（约 1–3ms），高频短命令循环有可测开销；靠 `+crt-static`、最小依赖压低。
- Ctrl+C 透传依赖「同控制台共享」；若未来需 `CREATE_NEW_PROCESS_GROUP`，须改 `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT)` 手动转发。
- 写 HKCU Path 必须 `get_raw_value` 保原类型（通常 `REG_EXPAND_SZ`），否则破坏 `%USERPROFILE%`；写前备份。
- `WM_SETTINGCHANGE` 仅影响新进程，须提示重开终端。
- pythonw shim 必须 GUI 子系统变体，否则闪黑窗。
- pip 装新脚本后需 `rehash` 才能全局调用（类 pyenv rehash）。
- 部分版本前缀匹配取最高 patch，可能与用户预期精确版本不符，须文档化。
- install_only 无 `python3.exe`，shim 不暴露 `python3`。

**PBS 来源**
- 匿名 GitHub API 限流 60 req/h；必须缓存 `pbs-index.json` + ETag/If-None-Match + 可选 `GITHUB_TOKEN`，`remaining==0` 明确报错。
- 必带 `User-Agent` 否则 403。
- `pgo-full.tar.zst`（full）需 zstd（C 工具链）；默认只走 `.tar.gz` 纯 Rust，full 设 `zstd-full` feature。
- Windows install_only 顶层是 `python\`（`python\python.exe`）；full 顶层是 `python\install\`，须按 flavor 选校验路径并规整布局。
- 同 xyz 跨多日期 release，须按 `release_date` 去重保最新。
- freethreaded 资产为复合后缀命名（`freethreaded+...`），须正则提取而非硬编码（见 §7.2 待定项）。
- digest 字段已自带 sha256，无需再下 SHA256SUMS。
- tar 解包遇长路径须 `\\?\` 前缀，并防 tar 穿越。

**python.org 来源**
- 卸载强依赖原 bootstrapper exe，必须缓存，丢失则按名重下。
- per-user + 自定义 TargetDir 仍可能弹 UAC；放 home 下规避但非 100%，须捕获退出码提示（可建议改 standalone）。
- 用户名含非 ASCII/空格的 TargetDir 可能触发边缘 bug，必要时回退 standalone。
- `index-windows.json` 偏向新版安装管理器，下载 exe 仍按 ftp 模板自拼并可 HEAD 校验。
- 3.13+ 的 `Include_launcher` 实际安装新版 Python Install Manager；`Include_launcher=0` 跳过它，不装 `py.exe`，须文档说明。
- `._pth` 默认内容随版本变化，`patch_pth` 须读实际内容做幂等替换。
- get-pip.py 对 EOL 版本走版本化 URL。
- 退出码 `0`/`3010` 判成功；强制 `InstallAllUsers=0`。
- 嵌入式 zip 不适合做 venv 基底，venv 子系统禁止以嵌入式版本为基底。

**venv / pip 镜像**
- PowerShell ExecutionPolicy 可能阻止 `Activate.ps1`，提示中附 `Set-ExecutionPolicy -Scope Process -Bypass`。
- 写全局 pip.ini 必须 rust-ini 保序合并 + 首次备份，不整文件重写。
- 清华两等价 URL，固定短域名并文档说明。
- `--in-project` 的 `.venv` 不被 `venv list` 列出，须文档明示。
- 镜像 URL 硬编码属维护风险，须可被裸 URL 覆盖。

**工程架构**
- zstd 需 cc 工具链（MSVC build tools）；默认不启用。
- MAX_PATH 260：全程 `\\?\` + manifest `longPathAware`，不依赖系统策略开关。
- `+crt-static` 静态链接 CRT，避免目标机缺 VCRUNTIME140.dll。
- 批量 install 部分失败须逐项报告，整体失败退出码（4/5）反映，不吞错。
- 单 crate 三 bin 共享依赖，shim 体积依赖 LTO/死代码消除控制；超标再拆 workspace。

### 9.2 未决问题（需实现前/中决策）

1. **freethreaded 资产精确后缀**：Windows PBS freethreaded 命名为复合形式（`freethreaded+pgo-install_only.tar.gz` 一类），**必须从实际 release JSON 正则提取**，不可硬编码。> 待定: 是否存在非 `install_only` 的 freethreaded Windows 变体、`+lto`/`+pgo` 组合是否随版本漂移。**决策点：枚举正则的 freethreaded 分支以「含 `freethreaded` 子串 + `install_only.tar.gz` 收尾」为准。**
2. **full 包目录规整**：`pgo-full.tar.zst` 顶层 `python\install\`，须把 `install\*` 提升到与 install_only 一致的 `python\` 布局，以复用同一 `Paths::python_exe`。> 待定: 提升策略与可能的符号链接/相对路径副作用。
3. **shim rehash 入口枚举范围**：默认仅核心入口（python/pythonw/pip/pip3）+ venv 入口；console_scripts 全量 rehash 由 `config.toml` 开关控制。**决策点：默认是否对 global/local 生效版本的 Scripts 全量 rehash（本 SPEC 倾向是，仅生效版本，不及全部已装版本）。**
4. **`PVM_VERSION` 会话级注入**：`pvm shell` 无法改父进程环境，依赖 `pvm init` 注入的 shell 函数。PowerShell/cmd/Git-Bash 各产出函数模板；cmd 无法定义持久函数，只能给 `set PVM_VERSION=` 提示。**决策点：cmd 仅给提示不提供函数（本 SPEC 倾向是）。**
5. **`-y/--yes` 定义位置**：必须二选一（全局 or 局部），不可同名重复定义。**决策点：统一用全局 `--yes`，删 `Uninstall` 局部 `-y`。**
6. **shim 二进制瘦身**：单 crate 共享 `[dependencies]`，若 shim 体积超标，改 workspace 拆 `pvm-core`。**决策点：本期单 crate，超标再拆。**

---

> 文档状态：本修订版已抽查核实 PBS 资产命名/digest 格式、python.org 安装器参数与 ftp URL 模板、`windows-sys 0.59` feature 名、各 crate 版本族，并修正了 `PbsFlavor::Full`→`PgoFull`（`pgo-full.tar.zst`）、删除冗余 `parse_sha256sums`/reqwest 备选、统一全局版本单一来源（`root\version`）、补全 `FlavorArg`/`Installer` 错误/`Paths::bin`/`pythonw_exe` 等接口缺口。仍以「> 待定:」标注 freethreaded 后缀提取、full 目录规整、`-y` 定义位置等需实现期最终敲定的细节。