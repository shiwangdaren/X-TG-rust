//! xAI Grok 兼容的 Chat Completions（`POST .../v1/chat/completions`，Bearer）。
//!
//! Grok 文档：<https://docs.x.ai/docs/api-reference#chat-completions>

use serde::Deserialize;
use xtg_core::AiJobConfig;

/// 去掉首尾空白、UTF-8 BOM、以及误粘贴的 `Bearer ` 前缀（避免 `Authorization: Bearer Bearer …`）。
fn sanitize_api_key(key: &str) -> String {
    let mut s = key.trim().trim_start_matches('\u{feff}').to_string();
    if let Some(rest) = s.strip_prefix("Bearer ") {
        s = rest.trim().to_string();
    } else if let Some(rest) = s.strip_prefix("bearer ") {
        s = rest.trim().to_string();
    }
    s
}

/// 规范 Chat Completions 根路径：常见误填 `https://api.x.ai` 未带 `/v1`。
fn normalize_chat_api_base(base: &str) -> String {
    let mut s = base.trim().trim_end_matches('/').to_string();
    if s.eq_ignore_ascii_case("https://api.x.ai") || s.eq_ignore_ascii_case("http://api.x.ai") {
        s.push_str("/v1");
    }
    s
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageBody,
}

#[derive(Deserialize)]
struct MessageBody {
    /// Grok/OpenAI 在少数情况下可能返回 null。
    #[serde(default)]
    content: Option<String>,
}

pub async fn translate_to_zh(
    http: &reqwest::Client,
    ai: &AiJobConfig,
    text: &str,
) -> Result<String, String> {
    let api_key = sanitize_api_key(&ai.api_key);
    if !ai.enabled || api_key.is_empty() {
        return Ok(text.to_string());
    }
    let base = normalize_chat_api_base(&ai.api_base);
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": ai.model,
        "temperature": 0.3,
        "messages": [{
            "role": "user",
            "content": format!("将以下推文内容翻译成简体中文，只输出译文，不要解释或引号：\n\n{}", text)
        }]
    });
    let res = http
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let body_bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status,
            String::from_utf8_lossy(&body_bytes)
        ));
    }
    let parsed: ChatCompletion =
        serde_json::from_slice(&body_bytes).map_err(|e| e.to_string())?;
    parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "API 未返回译文".to_string())
}
