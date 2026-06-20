//! 统一 HTTP agent：自动走环境变量 / Windows 系统代理。
//!
//! 解决直连 github.com / pypi.org 被墙、而用户代理（如 127.0.0.1:7890）可达的问题：
//! ureq 默认不一定走系统代理，这里显式探测代理并配置到全局 agent。

use std::sync::OnceLock;

/// 全局共享 agent（带代理探测）。clone 成本低（内部 Arc）。
pub fn agent() -> ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(build_agent).clone()
}

fn build_agent() -> ureq::Agent {
    // 代理串支持 http(s):// 与 socks5://（用户可用 ALL_PROXY=socks5://127.0.0.1:7890 指定）。
    match detect_proxy().and_then(|p| ureq::Proxy::new(&p).ok()) {
        Some(proxy) => {
            ureq::Agent::new_with_config(ureq::Agent::config_builder().proxy(Some(proxy)).build())
        }
        None => ureq::Agent::new_with_defaults(),
    }
}

/// 探测代理：环境变量优先，回退 Windows 系统代理（WinINET）。返回如 `http://127.0.0.1:7890`。
/// 探测代理：**默认直连**（适合 TUN 模式，让系统层接管）。
/// 仅 PVM_PROXY 控制：未设 / "direct" = 直连；"system" = 系统/环境代理；其它 = 自定义代理 URL。
pub fn detect_proxy() -> Option<String> {
    match std::env::var("PVM_PROXY") {
        Ok(v) => {
            let v = v.trim();
            if v.is_empty() || v.eq_ignore_ascii_case("direct") {
                None
            } else if v.eq_ignore_ascii_case("system") {
                system_proxy()
            } else {
                Some(normalize(v))
            }
        }
        Err(_) => None,
    }
}

/// 系统/环境代理（仅在 PVM_PROXY=system 时使用）。
fn system_proxy() -> Option<String> {
    for k in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(normalize(v));
            }
        }
    }
    #[cfg(windows)]
    {
        if let Some(p) = win_system_proxy() {
            return Some(p);
        }
    }
    None
}

fn normalize(v: &str) -> String {
    if v.contains("://") {
        v.to_string()
    } else {
        format!("http://{v}")
    }
}

#[cfg(windows)]
fn win_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let k = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enable: u32 = k.get_value("ProxyEnable").ok()?;
    if enable == 0 {
        return None;
    }
    let server: String = k.get_value("ProxyServer").ok()?;
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    // "host:port" 或 "http=host:port;https=host:port;ftp=..."
    let picked = if server.contains('=') {
        server
            .split(';')
            .find_map(|p| p.strip_prefix("https=").or_else(|| p.strip_prefix("http=")))
            .unwrap_or(server)
            .to_string()
    } else {
        server.to_string()
    };
    Some(normalize(&picked))
}
