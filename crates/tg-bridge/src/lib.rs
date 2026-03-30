//! Telegram 用户态（grammers MTProto）封装。

mod peer_resolve;

use async_trait::async_trait;
use grammers_client::media::InputMedia;
use grammers_client::message::InputMessage;
pub use grammers_client::client::{LoginToken, PasswordToken};
pub use grammers_client::SignInError;
use grammers_client::{Client, InvocationError};
use grammers_client::peer::User;
use grammers_session::types::PeerRef;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
pub use grammers_session::updates::UpdatesLike;
use peer_resolve::resolve_target_peer;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;
use xtg_core::{CoreError, DownloadedMedia, Post, Result, TgSink};

fn map_inv(e: InvocationError) -> CoreError {
    CoreError::Telegram(format!("{e:?}"))
}

/// 连接池 + 客户端句柄；`runner` 需在后台 `tokio::spawn`。
pub struct GrammersPool {
    pub client: Client,
    pub runner: grammers_mtsender::SenderPoolRunner,
    /// 与 [`grammers_client::Client::stream_updates`] 配对使用，仅能消费一次。
    pub updates: mpsc::UnboundedReceiver<UpdatesLike>,
}

impl GrammersPool {
    pub async fn connect(session_path: &Path, api_id: i32) -> Result<Self> {
        let session = Arc::new(
            SqliteSession::open(session_path)
                .await
                .map_err(|e| CoreError::Telegram(e.to_string()))?,
        );
        let SenderPool {
            runner,
            handle,
            updates,
        } = SenderPool::new(session, api_id);
        let client = Client::new(handle);
        Ok(GrammersPool {
            client,
            runner,
            updates,
        })
    }
}

pub async fn is_authorized(client: &Client) -> Result<bool> {
    client
        .is_authorized()
        .await
        .map_err(|e| CoreError::Telegram(format!("{e:?}")))
}

/// 请求短信/应用内验证码。
pub async fn request_login_code(
    client: &Client,
    phone: &str,
    api_hash: &str,
) -> Result<LoginToken> {
    client
        .request_login_code(phone, api_hash)
        .await
        .map_err(|e| CoreError::Telegram(format!("{e:?}")))
}

/// 使用验证码登录用户账号。
pub async fn sign_in_with_code(
    client: &Client,
    token: &LoginToken,
    code: &str,
) -> std::result::Result<User, grammers_client::SignInError> {
    client.sign_in(token, code).await
}

/// 2FA 密码。
pub async fn sign_in_with_password(
    client: &Client,
    token: PasswordToken,
    password: impl AsRef<[u8]>,
) -> std::result::Result<User, grammers_client::SignInError> {
    client.check_password(token, password).await
}

/// 投递实现：需已授权的 [`Client`]。
pub struct GrammersSink {
    client: Client,
}

impl GrammersSink {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn inner_client(&self) -> &Client {
        &self.client
    }

    /// 向已解析的会话发送与 [`TgSink::send_post`] 相同的内容（用于群内回复等）。
    pub async fn send_post_to_peer_ref(
        &self,
        peer_ref: PeerRef,
        post: &Post,
        local_media: &[DownloadedMedia],
        translated_text: Option<&str>,
    ) -> Result<()> {
        self.send_post_impl(peer_ref, post, local_media, translated_text)
            .await
    }
}

const TG_MAX_MSG: usize = 4096;

fn format_post_time_line_ms(ms: i64) -> String {
    use chrono::{FixedOffset, TimeZone, Utc};
    let utc = match Utc.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(dt, _) => dt,
        chrono::LocalResult::None => return format!("时间戳(ms): {ms}"),
    };
    let cn = FixedOffset::east_opt(8 * 3600).unwrap();
    let t = utc.with_timezone(&cn);
    format!(
        "发帖时间：{} (UTC+8)",
        t.format("%Y-%m-%d %H:%M:%S")
    )
}

fn chunk_text(s: &str) -> Vec<String> {
    if s.is_empty() {
        return vec![String::new()];
    }
    s.chars()
        .collect::<Vec<_>>()
        .chunks(TG_MAX_MSG)
        .map(|c| c.iter().collect())
        .collect()
}

impl GrammersSink {
    async fn send_post_impl(
        &self,
        peer_ref: PeerRef,
        post: &Post,
        local_media: &[DownloadedMedia],
        translated_text: Option<&str>,
    ) -> Result<()> {
        let main_text = translated_text.unwrap_or(post.text.as_str());
        let time_line = post
            .effective_time_ms()
            .map(|ms| format!("{}\n", format_post_time_line_ms(ms)))
            .unwrap_or_default();
        let reply_line = post
            .reply_to_url
            .as_ref()
            .map(|u| format!("【回复】\n源帖：{}\n", u))
            .unwrap_or_default();
        let mut header = format!(
            "@{} · {}\n{}{}{}",
            post.author_handle, post.id, time_line, reply_line, main_text
        );
        if let Some(u) = post.url.as_ref() {
            header.push_str(&format!("\n{}", u));
        }
        if !post.media.is_empty() {
            header.push_str("\n\n媒体（下载完成后补发文件）：");
            for m in &post.media {
                header.push_str(&format!("\n{}", m.url));
            }
        }
        let body = header;

        // 先发汉化正文与链接；再上传本地媒体（若有）。
        for part in chunk_text(&body) {
            if !part.is_empty() {
                self.client
                    .send_message(peer_ref, part)
                    .await
                    .map_err(map_inv)?;
            }
        }

        if local_media.is_empty() {
            debug!("sent post {} (text only)", post.id);
            return Ok(());
        }

        let images: Vec<_> = local_media
            .iter()
            .filter(|p| {
                p.mime
                    .as_ref()
                    .map(|m| m.starts_with("image/"))
                    .unwrap_or_else(|| {
                        p.path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| matches!(e, "jpg" | "jpeg" | "png" | "gif" | "webp"))
                            .unwrap_or(false)
                    })
            })
            .collect();

        let videos_or_docs: Vec<_> = local_media
            .iter()
            .filter(|p| {
                p.mime
                    .as_ref()
                    .map(|m| m.starts_with("video/") || m == "application/octet-stream")
                    .unwrap_or_else(|| {
                        p.path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| matches!(e, "mp4" | "webm" | "mov"))
                            .unwrap_or(false)
                    })
            })
            .collect();

        if !images.is_empty() {
            let mut album: Vec<InputMedia> = Vec::new();
            for dm in images.iter() {
                let uploaded = self
                    .client
                    .upload_file(&dm.path)
                    .await
                    .map_err(|e| CoreError::Telegram(e.to_string()))?;
                album.push(InputMedia::new().photo(uploaded));
            }
            self.client
                .send_album(peer_ref, album)
                .await
                .map_err(map_inv)?;
        } else if let Some(v) = videos_or_docs.first() {
            let uploaded = self
                .client
                .upload_file(&v.path)
                .await
                .map_err(|e| CoreError::Telegram(e.to_string()))?;
            let msg = InputMessage::new().document(uploaded);
            self.client
                .send_message(peer_ref, msg)
                .await
                .map_err(map_inv)?;
        } else {
            for dm in local_media.iter() {
                let uploaded = self
                    .client
                    .upload_file(&dm.path)
                    .await
                    .map_err(|e| CoreError::Telegram(e.to_string()))?;
                let msg = InputMessage::new().document(uploaded);
                self.client
                    .send_message(peer_ref, msg)
                    .await
                    .map_err(map_inv)?;
            }
        }

        debug!("sent post {}", post.id);
        Ok(())
    }
}

#[async_trait]
impl TgSink for GrammersSink {
    async fn send_post(
        &self,
        target: &str,
        post: &Post,
        local_media: &[DownloadedMedia],
        translated_text: Option<&str>,
    ) -> Result<()> {
        let peer = resolve_target_peer(&self.client, target)
            .await
            .map_err(map_inv)?;
        let peer_ref = peer
            .to_ref()
            .await
            .ok_or_else(|| CoreError::Telegram("peer ref missing".into()))?;
        self.send_post_impl(peer_ref, post, local_media, translated_text)
            .await
    }
}
