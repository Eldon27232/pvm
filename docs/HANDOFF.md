# pvm 项目交接文档

> 面向接手开发者。读完应能：构建运行、理解架构、避开已知坑、按既有模式扩展功能。
> 配套文档：`docs/SPEC.md`（CLI 技术规格）、`README.md`（用户向）。

## 1. 这是什么

**pvm** = Windows 平台的 Python **版本管理器 + 包管理器**，Rust 编写，提供：

- **CLI**：`pvm.exe`（+ `pvm-shim.exe` / `pvm-shimw.exe` 两个 shim 转发器）
- **GUI**：`pvm-gui.exe`（Tauri 2，前端原生 HTML/CSS/JS，无前端框架/构建链）

仓库：https://github.com/Eldon27232/pvm ｜ 根目录：`C:\Users\27232\Documents\pvm`
所有运行期数据在 `%USERPROFILE%\.pvm\`（versions / shims / venvs / cache / snapshots / config.toml 等）。

## 2. 构建与运行

需要 Rust stable（`x86_64-pc-windows-msvc`）。MSVC 工具链 + WebView2（Win11 自带）。

```powershell
# CLI（含 shim）
cargo build --release                 # → target\release\pvm.exe / pvm-shim.exe / pvm-shimw.exe

# GUI 开发期快速迭代（编译+开窗，加载 gui/dist 静态前端）
cargo run -p pvm-gui

# GUI 仅编译检查（最常用，快）
cargo build -p pvm-gui

# GUI 正式打包（单 exe + NSIS 安装包），需 tauri-cli
cargo install tauri-cli --version "^2" --locked
cd gui && cargo tauri build           # → target\release\pvm-gui.exe + bundle\nsis\*-setup.exe
```

**安装包带 CLI/shim 的机制**：`gui/tauri.conf.json` 的 `bundle.externalBin` 声明三个 sidecar，文件在 `gui/binaries/<name>-x86_64-pc-windows-msvc.exe`。打 release 前需先 `cargo build --release` 再把 `target/release/pvm*.exe` 复制成 sidecar 命名（见提交 `d353665`、各次 build 脚本）。`gui/binaries/` 已 gitignore。

前端是纯静态文件（`gui/dist/`），改了 `dist/` 必须重新 `cargo build`/`cargo tauri build` 才会重新嵌入二进制。

## 3. 架构

### 3.1 core 库（`src/`，crate `pvm`）

| 模块 | 职责 |
|---|---|
| `version` | 版本号/来源解析（`PythonVersion`、`Source{Standalone,Org}`、canonical id `cpython-x.y.z[-t]-{standalone\|org}`、`VersionSelector`） |
| `paths` | 所有磁盘路径从 root 派生；`python_exe`/`scripts_dir` 按来源选子路径 |
| `config` | `config.toml`（default_source / pip_mirror / proxy / ai_* / use_uv）。**注意 global 版本不在 config，存 `root\version` 文件** |
| `error` | 统一 `PvmError`（thiserror），每变体对应退出码 |
| `net` | **统一 HTTP agent**（ureq），代理探测见 §6.1 |
| `resolve` | 生效版本解析（PVM_VERSION > .python-version 向上找 > global）、selector→已装版本 |
| `download` | 多线程分块下载（HTTP Range + `seek_write` 并发 + 单线程回退 + 进度回调） |
| `archive` | 解压 tar.gz / tar.zst / zip |
| `source_pbs` | python-build-standalone：GitHub Releases 枚举 + 下载安装（**含 504 重试**） |
| `source_pyorg` | python.org：ftp 枚举 + 官方安装器静默安装 / 嵌入式 |
| `system` | 系统已装 Python 发现（注册表 PythonCore + py launcher）、`path_pythons`（PATH 冲突诊断） |
| `pkg` | **包管理核心**：纯 Rust 扫 site-packages 列包/读 METADATA、pip install/uninstall/freeze/outdated、流式安装、dry-run、PyPI 详情/搜索、健康评分、依赖图、uv 路由 |
| `snapshot` | 环境快照（freeze 存档）/ 克隆 |
| `pip` | pip 镜像配置（写 pip.ini）、内置镜像表 MIRRORS |
| `venv` | 虚拟环境创建/列出/删除 |
| `ai` | AI 诊断 + 找包（OpenAI `/chat/completions` 与 Anthropic `/v1/messages` 两种格式） |
| `osv` | OSV 漏洞批量扫描（api.osv.dev） |
| `shim` / `winpath` | **（已停用接管）** 历史 shim 生成/PATH 注入代码保留；现仅 `shim::cleanup_legacy` 用于清理旧接管。pvm 不再修改系统 PATH，详见 §3.4 |
| `cli` / `commands` | clap 定义 + CLI 命令分发（`src/main.rs` 是 CLI 入口；`bin/shim.rs`、`bin/shimw.rs` 是 shim） |

### 3.2 GUI（`gui/`，crate `pvm-gui`）

- `src/main.rs`：Tauri 入口，启动注入代理 env，注册 **48 个命令**（invoke_handler）
- `src/commands.rs`：所有 `#[tauri::command]`，**全部薄封装 core**，长任务用 `spawn_blocking` 或 `std::thread::spawn`+事件
- `dist/`：`index.html` / `style.css` / `app.js`（全部逻辑）/ `i18n.js`（中英文案）
- `capabilities/default.json`：Tauri 权限；`icons/`：图标（`gen_icon.py` 可重生成）；`tauri.conf.json`：窗口/bundle 配置

### 3.3 前后端契约

前端 `window.__TAURI__.core.invoke('命令名', {参数})`。**Tauri 自动 snake_case↔camelCase**：后端 `py_exe` ↔ 前端 `pyExe`，`req_file`↔`reqFile` 等。事件用 `window.__TAURI__.event.listen`（如 `install://progress`、`batch://line`）。

app.js 关键约定：每个面板一个 `renderXxx(c)` 函数（installed/packages/install/venv/mirror/settings），`render()` 按 `state.nav` 分发；列表/解释器缓存用 **localStorage**（key `pkgs:<py_exe>`、`interps`、`lastPy`），`prewarm()` 启动预热；模态用全局 `openModal/closeModal`。

### 3.4 架构变更：pvm 不再接管系统 PATH（重要）

早期版本仿 pyenv：把 `~/.pvm/shims` 注入用户 PATH 最前，所有 `python`/`pip` 经 shim 转发——这会**接管系统 python**，导致其它依赖系统 Python 的项目（无 pvm 生效版本时）被 shim 拦截报错。

现已改为**纯管理器**：
- `init` 不再注入 PATH、不部署 shims，并调 `shim::cleanup_legacy` 清理历史接管（移除 PATH 项 + 删 `~/.pvm/shims`）。
- `pvm global/local`、装包、建 venv 都不再 `rehash`。`global` 退化为「GUI/venv 的默认解释器记忆」，不影响系统命令行。
- pvm 下载的版本就是 `~/.pvm/versions\...` 下的普通 python.exe，与系统安装平等。使用方式：GUI「打开终端」（把所选解释器加进**本会话** PATH）、建 venv，或用户自行加 PATH。
- 代价：放弃 `.python-version` 目录级**自动**切换（该能力依赖 shim 拦截 cwd）。
- shim 子系统代码（`shim.rs`/`bin/shim.rs`/`winpath` 注入函数）保留但不再走生产路径，后续可清理。
- `pvm rehash` 命令保留，语义改为「清理 pvm 对 PATH 的接管」。GUI 设置页诊断有「PATH 接管」告警，检测到残留时提示点初始化清理。

## 4. 功能总览（提交演进见 `git log`）

版本管理（双源 standalone/cpython、全局/局部切换、shim、系统识别）｜包管理（秒级列包、富详情、PyPI 搜索、AI 找包、dry-run、批量+流式日志、过时检测、requirements 导入导出）｜环境（venv、脚手架、快照/克隆、开激活终端）｜健康安全（OSV 扫描、健康评分、依赖图、PATH 诊断）｜加速智能（uv 路由、AI 诊断、pip 镜像）｜体验（中英 i18n、深浅主题、代理可配）。

## 5. 扩展指引（加一个新功能的标准路径）

1. **core**：在合适模块加 `pub fn`（纯逻辑，返回 `Result<T, PvmError>`，不打印——见 §6.2）。
2. **GUI 命令**：`gui/src/commands.rs` 加 `#[tauri::command]` 薄封装；网络/子进程类用 `spawn_blocking`。
3. **注册**：`gui/src/main.rs` 的 `generate_handler!` 加命令名。
4. **前端**：`app.js` 加 UI + `invoke`；`i18n.js` 加中英文案两份。
5. **验证**：`cargo build -p pvm-gui` + `node --check gui/dist/app.js` + `node --check gui/dist/i18n.js`。

## 6. 已知坑与约束（务必先读）

### 6.1 网络 / standalone（最重要）
- `net::detect_proxy` 读 **`PVM_PROXY`**：未设/`direct`=**直连（默认，适合 TUN）**；`system`=环境变量/Windows 系统代理；其它=自定义 URL。GUI 设置写 `config.proxy`，main 启动注入 env，**改后需重启**。
- **python-build-standalone 在本用户网络不可用**：`api.github.com` 经其 FlClash 节点稳定 504（http 代理 / socks5 / 直连 / 重试 4 次全试过；同节点 python.org/PyPI 正常）。这是节点对该域的分流问题，pvm 无解 —— **用 python.org(cpython) 源**。已加重试 + 友好报错。

### 6.2 GUI 无 console → 禁止 `println!`
release 的 GUI 是 `windows_subsystem="windows"`，无 stdout，`println!` 会 panic。**core 函数不要直接打印**，返回值由调用方决定输出（参考 `venv::activation_hint`）。

### 6.3 子进程编码
中文 Windows 下 pip/python 子进程输出可能是 cp936。`pkg::pip()` / `run_py()` 统一注入 `PYTHONUTF8=1` + `PYTHONIOENCODING=utf-8`，新加子进程调用照做。

### 6.4 工具链 / 流程
- **CLI 提交信息用中文** → 用 git-bash（Bash 工具）提交，避免 PowerShell 编码乱码。
- `git push` 偶发 `schannel: failed to receive handshake`（网络），**重试即过**。
- 文件多为 LF，git 会警告 CRLF 转换，无害。
- **computer-use resolver 不识别新编译的 exe**，无法自动截图 GUI；渲染正确性靠 `cargo build` + `node --check` + 数据源验证 + 人工实测。
- ureq 3 API：`Agent::config_builder().proxy(Some(Proxy::new(url)?)).build()` → `Agent::new_with_config`；http 代理无需 feature，socks5 需 `socks-proxy` feature（已启用）；`send_json` 需 `json` feature（已启用）。

### 6.5 AI / uv 需用户侧前置
- AI 功能（诊断/找包）需用户在设置页配 Base URL + Key + Model（key 存 `config.toml` 明文，仅本地）。
- uv 加速需机器已装 uv（`pip install uv` 或 `winget install astral-sh.uv`），`pkg::uv_path()` 用 `where uv` 检测。

## 7. 未做 / 后续可选（来自需求决策清单③档剩余）

跨机环境同步（Gist/WebDAV）｜一键注册到 IDE/Jupyter（写 VS Code 解释器、装 ipykernel）｜SBOM 导出（CycloneDX/SPDX）｜磁盘瘦身中心（pip cache + __pycache__ + 残留 dist-info）｜conda 环境识别｜依赖关系**图形化**（当前是列表，可上 SVG/力导向）｜standalone 的 GitHub 镜像加速（绕开 api.github.com，未验证可行性）。
