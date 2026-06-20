# pvm —— Windows Python 版本管理器

用 Rust 编写的、**仅面向 Windows x86_64** 的 Python 多版本管理器，对标 pyenv-win / nvm-windows，
形态为单个 `pvm.exe` + 一组原生 shim。

## 特性

- **多版本安装 / 卸载 / 全局切换**，远程实时枚举可装版本。
- **局部版本**：按目录读取 `.python-version` 自动切换（pyenv 风格）。
- **原生 Rust shim**（非 .bat）：确定性版本解析、参数与退出码透传、Ctrl+C 透传。
- **双解释器来源**：
  - `python-build-standalone`（默认，解压即用、内含 pip、无需管理员、无注册表写入）；
  - `python.org` 官方安装器（per-user 静默安装）。
  - 同一版本号双来源可并存。
- **虚拟环境管理**：基于选定版本创建 / 列出 / 删除 venv。
- **pip 国内镜像加速**：内置清华 / 阿里 / 中科大 / 腾讯 / 华为镜像，写入 `pip.ini`。
- 单 exe 分发，静态链接 CRT（`+crt-static`），目标机无需 VC++ 运行库。

## 系统要求

- Windows 10/11 x86_64。
- 构建需 Rust（stable，MSVC 工具链）。运行无额外依赖。

## 构建

```powershell
cargo build --release
# 产物：target\release\pvm.exe / pvm-shim.exe / pvm-shimw.exe
```

可选启用 `--flavor full`（pgo-full.tar.zst，需 C 工具链）：

```powershell
cargo build --release --features zstd-full
```

## 安装与初始化

把三个 exe 放到同一目录，运行 `init` 建立目录结构、安装 shim 模板、并把 shims 目录加入用户 PATH：

```powershell
pvm init
# 重开终端使 PATH 生效
```

## 命令速查

| 命令 | 说明 |
|---|---|
| `pvm install 3.12` | 安装某版本（默认 standalone；`--source cpython` 用 python.org） |
| `pvm install 3.12 -g` | 安装并设为全局 |
| `pvm install 3.13t --freethreaded` | 安装 free-threaded 变体（仅 standalone） |
| `pvm uninstall 3.12.7@org` | 卸载（`@org` / `@standalone` 消歧） |
| `pvm list` / `pvm ls` | 列出已安装（`*` 标记全局） |
| `pvm ls-remote [--source cpython]` | 列出远程可装版本 |
| `pvm global 3.12` / `pvm use 3.12` | 设置 / 查看全局版本 |
| `pvm local 3.11` | 写当前目录 `.python-version` |
| `pvm shell 3.13` | 打印会话级 `PVM_VERSION` 设置命令 |
| `pvm which [pip]` | 打印 shim 解析到的真实 exe |
| `pvm current` / `pvm version` | 当前生效版本与来源 |
| `pvm exec -- python -V` | 用生效版本临时运行命令 |
| `pvm venv create web --python 3.12 --mirror tuna` | 创建 venv |
| `pvm venv list / remove / path / which / activate` | venv 管理 |
| `pvm pip-mirror set tuna` | 设置 pip 镜像（`list` / `show` / `reset`） |
| `pvm rehash` | 重建 shim |
| `pvm doctor` | 环境诊断 |
| `pvm root` | 打印根目录 |

版本选择符支持：`3` / `3.12` / `3.12.7` / `3.12.7@org` / `3.13t` / `latest` / 完整 canonical id。

## 双来源命名

磁盘 canonical id：`cpython-<x.y.z>[t]-<standalone|org>`，例 `cpython-3.12.7-standalone`、`cpython-3.12.7-org`。
CLI `--source` 取值：`standalone` 或 `cpython`（`cpython` 即 python.org 官方）。

## 目录布局（`%USERPROFILE%\.pvm`）

```
.pvm\
├─ version              # 全局默认版本（canonical id）
├─ config.toml          # 配置（默认来源、pip 镜像等）
├─ bin\                 # pvm.exe 与 shim 模板
├─ shims\               # 加入用户 PATH，python/pip/... 转发器
├─ versions\<id>\       # 各 Python 版本
├─ venvs\<name>\        # 集中式 venv
├─ cache\               # 下载缓存、远程枚举缓存
├─ logs\                # python.org 安装器日志
└─ backup\              # 改 PATH 前的备份
```

可用 `--root <DIR>` 或 `PVM_ROOT` 覆盖根目录。

## 已知限制 / 注意事项

- 仅支持 Windows x86_64（不含 32 位 / ARM64 / Linux / macOS）。
- GitHub Releases 匿名 API 限流 60 次/小时；设置 `GITHUB_TOKEN` 可提升到 5000 次/小时。
- `--flavor full`（pgo-full.tar.zst）需以 `--features zstd-full` 构建。
- python.org 嵌入式包为受限备选，不适合作 venv 基底。
- 改 PATH 后需重开终端才生效（仅影响新进程）。
- shim 不暴露 `python3`（standalone install_only 不含 `python3.exe`）。

## 设计文档

完整技术规格见 [docs/SPEC.md](docs/SPEC.md)。

## 开发

```powershell
cargo check --all-targets
cargo test
cargo clippy --all-targets
```
