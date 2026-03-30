//! 核心类型与抓取/投递抽象。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// 单条推文（或占位假数据）的统一表示，供 pipeline 使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    /// 推文 id（十进制数字字符串，与 X 一致）
    pub id: String,
    pub author_handle: String,
    pub text: String,
    pub media: Vec<MediaItem>,
    pub url: Option<String>,
    /// X API `created_at` 解析后的 Unix 毫秒；未设置时用推文 id（snowflake）推算。
    #[serde(default)]
    pub posted_at_ms: Option<i64>,
    /// 若为回复他人推文，指向源帖的链接（如 `https://x.com/i/status/…`）。
    #[serde(default)]
    pub reply_to_url: Option<String>,
}

impl Post {
    /// 用于展示的发帖时间（毫秒 Unix）：优先 API 时间，否则由 id 推算。
    pub fn effective_time_ms(&self) -> Option<i64> {
        self.posted_at_ms
            .or_else(|| tweet_id_to_unix_ms(&self.id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub url: String,
    pub kind: MediaKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Gif,
    Unknown,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("fetch: {0}")]
    Fetch(String),
    #[error("telegram: {0}")]
    Telegram(String),
    #[error("storage: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// 从 X 侧拉取某 handle 的最新帖子（实现可为浏览器会话或假数据）。
#[async_trait]
pub trait XSource: Send + Sync {
    async fn fetch_latest(&self, handle: &str) -> Result<Vec<Post>>;
}

/// 以 Telegram 用户身份投递帖子（媒体已由上层下载到本地路径）。
#[async_trait]
pub trait TgSink: Send + Sync {
    /// `target` 为 @username、username，或 Bot API 格式的数字 id（需在会话中有对话缓存）。
    /// `translated_text` 为 `Some` 时优先作为正文（汉化）；无媒体时直接发该文本，有媒体时先发汉化+链接再发文件。
    async fn send_post(
        &self,
        target: &str,
        post: &Post,
        local_media: &[DownloadedMedia],
        translated_text: Option<&str>,
    ) -> Result<()>;
}

/// xAI Grok Chat Completions（`POST …/v1/chat/completions`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiJobConfig {
    pub enabled: bool,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
}

impl Default for AiJobConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_base: "https://api.x.ai/v1".to_string(),
            api_key: String::new(),
            model: "grok-3-mini".to_string(),
        }
    }
}

/// 运行时可调度的任务配置快照（由 GUI/配置文件填充）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobConfig {
    pub x_handles: Vec<String>,
    /// 每行一个：@频道、群组用户名或数字 id；轮询成功帖会依次发往每个目标。
    pub tg_targets: Vec<String>,
    /// 轮询周期间隔（秒，可小数，例如 0.1）。
    pub poll_interval_secs: f64,
    pub max_media_bytes: u64,
    /// 点击「开始轮询」时的 Unix 毫秒时间；只转发推文 snowflake 时间不早于此。
    pub poll_started_at_ms: i64,
    pub ai: AiJobConfig,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            x_handles: vec![],
            tg_targets: vec![],
            poll_interval_secs: 60.0,
            max_media_bytes: 50 * 1024 * 1024,
            poll_started_at_ms: 0,
            ai: AiJobConfig::default(),
        }
    }
}

/// 由推文数字 id（snowflake）估算发布时间（毫秒 Unix）。用于「仅轮询开始之后发布的帖」。
pub fn tweet_id_to_unix_ms(id: &str) -> Option<i64> {
    let n: u64 = id.parse().ok()?;
    Some(((n >> 22) as i64).saturating_add(1288834974657))
}

/// 下载结果：本地路径 + 可选 MIME。
#[derive(Debug, Clone)]
pub struct DownloadedMedia {
    pub path: PathBuf,
    pub mime: Option<String>,
}
