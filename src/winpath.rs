//! Windows 专用：长路径前缀、用户 PATH 注入（保留 REG_EXPAND_SZ 类型）、shim 的 Ctrl+C 处理器。

use crate::error::{PvmError, Result};
use crate::paths::Paths;
use std::path::{Path, PathBuf};

/// 加 \\?\ 前缀规避 MAX_PATH（仅对绝对、非 UNC、未加前缀的路径）。
pub fn long_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s.starts_with(r"\\") {
        return p.to_path_buf(); // 已是 \\?\ 或 UNC，保持原样
    }
    if p.is_absolute() {
        PathBuf::from(format!(r"\\?\{s}"))
    } else {
        p.to_path_buf()
    }
}

#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ};
#[cfg(windows)]
use winreg::types::FromRegValue;
#[cfg(windows)]
use winreg::{RegKey, RegValue};

#[cfg(windows)]
fn open_env() -> Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|e| PvmError::Win(format!("打开 HKCU\\Environment 失败: {e}")))
}

/// 把 &str 编码为带 NUL 终止的 UTF-16LE 字节，用于写回 REG_EXPAND_SZ/REG_SZ。
#[cfg(windows)]
fn utf16le_bytes(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&[0, 0]);
    v
}

/// 读取用户 Path（当前值, 原始注册表类型）。不存在则空 + REG_EXPAND_SZ。
#[cfg(windows)]
fn read_user_path(env: &RegKey) -> (String, winreg::enums::RegType) {
    match env.get_raw_value("Path") {
        Ok(rv) => {
            let s = String::from_reg_value(&rv).unwrap_or_default();
            (s, rv.vtype)
        }
        Err(_) => (String::new(), REG_EXPAND_SZ),
    }
}

#[cfg(windows)]
fn write_user_path(env: &RegKey, value: &str, vtype: winreg::enums::RegType) -> Result<()> {
    let rv = RegValue {
        bytes: utf16le_bytes(value),
        vtype,
    };
    env.set_raw_value("Path", &rv)
        .map_err(|e| PvmError::Win(format!("写入用户 Path 失败: {e}")))
}

/// 把 shims 目录前插到用户 Path（去重、保原 vtype），首次写前备份，并广播 WM_SETTINGCHANGE。
#[cfg(windows)]
pub fn prepend_shims_to_user_path(shims_dir: &str, paths: &Paths) -> Result<()> {
    let env = open_env()?;
    let (old, vtype) = read_user_path(&env);

    let bak = paths.backup().join("user-path.bak");
    if !bak.exists() {
        std::fs::create_dir_all(paths.backup())?;
        std::fs::write(&bak, &old)?;
    }

    let mut parts: Vec<&str> = old.split(';').filter(|s| !s.is_empty()).collect();
    parts.retain(|p| !p.eq_ignore_ascii_case(shims_dir));
    let mut new_parts: Vec<String> = Vec::with_capacity(parts.len() + 1);
    new_parts.push(shims_dir.to_string());
    new_parts.extend(parts.into_iter().map(|s| s.to_string()));
    let new_val = new_parts.join(";");

    write_user_path(&env, &new_val, vtype)?;
    broadcast_setting_change();
    Ok(())
}

/// 从用户 Path 移除 shims 目录。
#[cfg(windows)]
pub fn remove_shims_from_user_path(shims_dir: &str) -> Result<()> {
    let env = open_env()?;
    let (old, vtype) = read_user_path(&env);
    let new_val = old
        .split(';')
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(shims_dir))
        .collect::<Vec<_>>()
        .join(";");
    write_user_path(&env, &new_val, vtype)?;
    broadcast_setting_change();
    Ok(())
}

/// 广播环境变量变更，使新开进程看到更新后的 PATH（不影响已运行进程）。
#[cfg(windows)]
fn broadcast_setting_change() {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let env_w: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as HWND,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            env_w.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

/// shim 专用：安装返回 TRUE 的空 Ctrl 处理器，让子进程接管 Ctrl+C/Break。
///
/// # Safety
/// 修改进程级控制台控制处理器，应在 shim 启动早期、单线程上下文调用一次。
#[cfg(windows)]
pub unsafe fn install_noop_ctrl_handler() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    // windows-sys 0.61 起 BOOL 即 i32（不再单独导出 Foundation::BOOL 别名）
    unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
        1 // TRUE：声明已处理，阻止默认处理器终止本进程；信号自然传到子进程
    }
    SetConsoleCtrlHandler(Some(handler), 1);
}
