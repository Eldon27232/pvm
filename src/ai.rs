//! AI 诊断：把 pip 错误发给用户配置的 LLM，返回诊断与修复建议。
//! 同时支持 OpenAI 兼容格式（/chat/completions）与 Anthropic 格式（/v1/messages）。

use crate::error::{PvmError, Result};
use std::io::Read;

pub struct AiConfig {
    pub provider: String, // "openai" | "anthropic"
    pub base_url: String,
    pub key: String,
    pub model: String,
}

const SYSTEM: &str = "你是 Python 包安装错误诊断助手。根据 pip 的报错输出，用简体中文简洁给出：1) 最可能的根因；2) 具体可执行的修复步骤（命令或操作）。直接给结论，不要寒暄。";

pub fn diagnose(cfg: &AiConfig, error_log: &str) -> Result<String> {
    let clipped: String = error_log.chars().take(4000).collect();
    let user = format!("pip 安装/操作失败，错误输出如下：\n```\n{clipped}\n```");
    if cfg.provider.eq_ignore_ascii_case("anthropic") {
        call_anthropic(cfg, &user)
    } else {
        call_openai(cfg, &user)
    }
}

fn call_openai(cfg: &AiConfig, user: &str) -> Result<String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": user }
        ],
        "temperature": 0.2
    });
    let resp = crate::net::agent()
        .post(&url)
        .header("Authorization", &format!("Bearer {}", cfg.key))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| PvmError::Http(format!("AI 请求失败: {e}")))?;
    let val = read_json(resp)?;
    val.pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| PvmError::Http(format!("AI 响应解析失败: {val}")))
}

fn call_anthropic(cfg: &AiConfig, user: &str) -> Result<String> {
    let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": cfg.model,
        "max_tokens": 1024,
        "system": SYSTEM,
        "messages": [{ "role": "user", "content": user }]
    });
    let resp = crate::net::agent()
        .post(&url)
        .header("x-api-key", &cfg.key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| PvmError::Http(format!("AI 请求失败: {e}")))?;
    let val = read_json(resp)?;
    val.pointer("/content/0/text")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| PvmError::Http(format!("AI 响应解析失败: {val}")))
}

fn read_json(resp: ureq::http::Response<ureq::Body>) -> Result<serde_json::Value> {
    let mut resp = resp;
    let mut s = String::new();
    resp.body_mut()
        .as_reader()
        .read_to_string(&mut s)
        .map_err(|e| PvmError::Http(e.to_string()))?;
    serde_json::from_str(&s).map_err(|e| {
        let head: String = s.chars().take(200).collect();
        PvmError::Http(format!("解析 AI 响应失败: {e}; 原文: {head}"))
    })
}
