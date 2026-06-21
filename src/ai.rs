//! AI 能力：诊断 pip 报错、自然语言找包、通用对话。
//! 同时支持 OpenAI 兼容格式（/chat/completions）与 Anthropic 格式（/v1/messages）。

use crate::error::{PvmError, Result};
use std::io::Read;

pub struct AiConfig {
    pub provider: String, // "openai" | "anthropic"
    pub base_url: String,
    pub key: String,
    pub model: String,
}

/// 一条对话消息。role 取 "user" | "assistant"（system 由各功能单独传入）。
#[derive(serde::Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

const SYSTEM: &str = "你是 Python 包安装错误诊断助手。根据 pip 的报错输出，用简体中文简洁给出：1) 最可能的根因；2) 具体可执行的修复步骤（命令或操作）。直接给结论，不要寒暄。";

const FIND_SYSTEM: &str = "你是 Python 包推荐助手。根据用户需求，推荐 3-6 个 PyPI 上合适、活跃的包。只返回 JSON 数组，格式 [{\"name\":\"包名\",\"reason\":\"一句话推荐理由\"}]，不要任何额外文字或代码块标记。";

const CHAT_SYSTEM: &str = "你是 pvm 的内置助手。pvm 是 Windows 平台的 Python 版本管理器 + 包管理器。回答聚焦 Python 版本管理、虚拟环境、pip/包管理、依赖与报错排查等开发问题，用简体中文，简洁直接、给可执行步骤；涉及命令时用代码块。不确定就说明，不要编造。";

/// 诊断 pip 报错。
pub fn diagnose(cfg: &AiConfig, error_log: &str) -> Result<String> {
    let clipped: String = error_log.chars().take(4000).collect();
    let user = format!("pip 安装/操作失败，错误输出如下：\n```\n{clipped}\n```");
    dispatch(cfg, SYSTEM, &[user_msg(user)])
}

/// 自然语言找包：让 LLM 推荐合适的 PyPI 包，返回 (name, reason)。
pub fn find_packages(cfg: &AiConfig, query: &str) -> Result<Vec<(String, String)>> {
    let user = format!("需求：{query}");
    let answer = dispatch(cfg, FIND_SYSTEM, &[user_msg(user)])?;
    parse_recommendations(&answer)
}

/// 通用多轮对话：history 含 user/assistant 交替消息，返回助手回复。
pub fn chat(cfg: &AiConfig, history: &[ChatMessage]) -> Result<String> {
    if history.is_empty() {
        return Err(PvmError::Http("对话内容为空".into()));
    }
    dispatch(cfg, CHAT_SYSTEM, history)
}

fn user_msg(content: String) -> ChatMessage {
    ChatMessage {
        role: "user".into(),
        content,
    }
}

fn dispatch(cfg: &AiConfig, system: &str, history: &[ChatMessage]) -> Result<String> {
    if cfg.provider.eq_ignore_ascii_case("anthropic") {
        call_anthropic(cfg, system, history)
    } else {
        call_openai(cfg, system, history)
    }
}

fn parse_recommendations(text: &str) -> Result<Vec<(String, String)>> {
    let start = text.find('[');
    let end = text.rfind(']');
    let json = match (start, end) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => text,
    };
    let arr: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| {
        let head: String = text.chars().take(200).collect();
        PvmError::Http(format!("解析 AI 推荐失败: {e}; 原文: {head}"))
    })?;
    Ok(arr
        .into_iter()
        .filter_map(|v| {
            let name = v.get("name")?.as_str()?.to_string();
            let reason = v
                .get("reason")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            Some((name, reason))
        })
        .collect())
}

fn call_openai(cfg: &AiConfig, system: &str, history: &[ChatMessage]) -> Result<String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let mut messages = vec![serde_json::json!({ "role": "system", "content": system })];
    for m in history {
        messages.push(serde_json::json!({ "role": m.role, "content": m.content }));
    }
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": 0.3
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

fn call_anthropic(cfg: &AiConfig, system: &str, history: &[ChatMessage]) -> Result<String> {
    let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
    let messages: Vec<serde_json::Value> = history
        .iter()
        .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
        .collect();
    let body = serde_json::json!({
        "model": cfg.model,
        "max_tokens": 2048,
        "system": system,
        "messages": messages
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
