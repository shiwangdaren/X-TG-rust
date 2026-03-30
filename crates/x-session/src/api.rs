//! Twitter / X API v2（Bearer Token，应用仅访问）拉取用户时间线。

use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;
use xtg_core::{CoreError, MediaItem, MediaKind, Post, Result, XSource};

fn millis_from_twitter_created_at(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// 使用 [Twitter API v2](https://developer.twitter.com/en/docs/twitter-api) 的 Bearer Token（OAuth 2.0 App-only）抓取用户最近推文。
///
/// 需在 [X Developer Portal](https://developer.twitter.com/) 创建应用并具备访问用户时间线所需权限（通常需 Elevated 等）。
pub struct TwitterApiV2Source {
    http: reqwest::Client,
    bearer_token: String,
    api_base: String,
}

impl TwitterApiV2Source {
    pub fn new(bearer_token: String, api_base: Option<String>) -> Self {
        let api_base = api_base
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://api.twitter.com".to_string());
        let api_base = api_base.trim_end_matches('/').to_string();
        Self {
            http: reqwest::Client::new(),
            bearer_token,
            api_base,
        }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header(
            "Authorization",
            format!("Bearer {}", self.bearer_token.trim()),
        )
    }

    async fn user_id_by_username(&self, username: &str) -> Result<String> {
        let url = format!(
            "{}/2/users/by/username/{}",
            self.api_base,
            urlencoding::encode(username)
        );
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;

        if !status.is_success() {
            return Err(CoreError::Fetch(format!(
                "X API 用户查询 HTTP {}: {}",
                status.as_u16(),
                body.chars().take(500).collect::<String>()
            )));
        }

        let v: UserByUsernameResp = serde_json::from_str(&body).map_err(|e| {
            CoreError::Fetch(format!("解析用户 JSON: {e}; body={}", body.chars().take(200).collect::<String>()))
        })?;

        v.data
            .map(|d| d.id)
            .ok_or_else(|| CoreError::Fetch("X API：未找到该用户名或无权访问".into()))
    }

    async fn user_tweets(&self, user_id: &str, handle: &str) -> Result<Vec<Post>> {
        let url = format!("{}/2/users/{}/tweets", self.api_base, user_id);
        let query = [
            ("max_results", "15"),
            (
                "tweet.fields",
                "created_at,attachments,referenced_tweets",
            ),
            ("expansions", "attachments.media_keys"),
            (
                "media.fields",
                "type,url,preview_image_url,variants",
            ),
        ];

        let resp = self
            .auth(self.http.get(&url).query(&query))
            .send()
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;

        if !status.is_success() {
            return Err(CoreError::Fetch(format!(
                "X API 时间线 HTTP {}: {}",
                status.as_u16(),
                body.chars().take(500).collect::<String>()
            )));
        }

        let v: TweetsResp = serde_json::from_str(&body).map_err(|e| {
            CoreError::Fetch(format!("解析时间线 JSON: {e}; body={}", body.chars().take(200).collect::<String>()))
        })?;

        let tweets = v.data.unwrap_or_default();
        let media_map = build_media_map(v.includes);

        let mut posts = Vec::new();
        for tw in tweets {
            let media = media_for_tweet(&tw, &media_map);
            let url = Some(format!("https://x.com/{}/status/{}", handle, tw.id));
            let reply_to_url = tw.reply_to_id().map(|id| {
                format!("https://x.com/i/status/{}", id)
            });
            posts.push(Post {
                id: tw.id,
                author_handle: handle.to_string(),
                text: tw.text,
                media,
                url,
                posted_at_ms: tw
                    .created_at
                    .as_deref()
                    .and_then(millis_from_twitter_created_at),
                reply_to_url,
            });
        }

        Ok(posts)
    }
}

#[async_trait]
impl XSource for TwitterApiV2Source {
    async fn fetch_latest(&self, handle: &str) -> Result<Vec<Post>> {
        let handle = handle.trim().trim_start_matches('@');
        if handle.is_empty() {
            return Err(CoreError::Fetch("empty handle".into()));
        }
        if self.bearer_token.trim().is_empty() {
            return Err(CoreError::Fetch("未配置 X API Bearer Token".into()));
        }

        let uid = self.user_id_by_username(handle).await?;
        let mut posts = self.user_tweets(&uid, handle).await?;

        posts.sort_by(|a, b| {
            let na: u128 = a.id.parse().unwrap_or(0);
            let nb: u128 = b.id.parse().unwrap_or(0);
            na.cmp(&nb)
        });

        if posts.is_empty() {
            warn!("X API：@{handle} 时间线无返回推文（可能无发帖或权限不足）");
        }

        Ok(posts)
    }
}

fn build_media_map(includes: Option<Includes>) -> std::collections::HashMap<String, MediaObj> {
    let mut m = std::collections::HashMap::new();
    if let Some(inc) = includes {
        if let Some(list) = inc.media {
            for mo in list {
                m.insert(mo.media_key.clone(), mo);
            }
        }
    }
    m
}

fn media_for_tweet(tw: &Tweet, media_map: &std::collections::HashMap<String, MediaObj>) -> Vec<MediaItem> {
    let Some(att) = &tw.attachments else {
        return vec![];
    };
    let mut out = Vec::new();
    for key in &att.media_keys {
        let Some(mo) = media_map.get(key) else {
            continue;
        };
        if let Some(item) = media_obj_to_item(mo) {
            out.push(item);
        }
    }
    out
}

fn media_obj_to_item(mo: &MediaObj) -> Option<MediaItem> {
    let t = mo.media_type.as_str();
    match t {
        "photo" => {
            let url = mo.url.clone()?;
            Some(MediaItem {
                url,
                kind: MediaKind::Image,
            })
        }
        "video" | "animated_gif" => {
            let kind = if t == "animated_gif" {
                MediaKind::Gif
            } else {
                MediaKind::Video
            };
            let url = best_video_url(mo.variants.as_ref())?;
            Some(MediaItem { url, kind })
        }
        _ => mo.url.clone().map(|url| MediaItem {
            url,
            kind: MediaKind::Unknown,
        }),
    }
}

fn best_video_url(variants: Option<&Vec<Variant>>) -> Option<String> {
    let vs = variants?;
    vs.iter()
        .filter(|v| v.content_type.as_deref() == Some("video/mp4"))
        .filter_map(|v| {
            let u = v.url.as_ref()?;
            Some((v.bitrate.unwrap_or(0), u.clone()))
        })
        .max_by_key(|(b, _)| *b)
        .map(|(_, u)| u)
        .or_else(|| {
            vs.iter().find_map(|v| v.url.clone())
        })
}

#[derive(Deserialize)]
struct UserByUsernameResp {
    data: Option<UserData>,
}

#[derive(Deserialize)]
struct UserData {
    id: String,
}

#[derive(Deserialize)]
struct TweetsResp {
    data: Option<Vec<Tweet>>,
    includes: Option<Includes>,
}

#[derive(Deserialize)]
struct Includes {
    media: Option<Vec<MediaObj>>,
}

#[derive(Deserialize)]
struct Tweet {
    id: String,
    text: String,
    #[serde(default)]
    created_at: Option<String>,
    attachments: Option<Attachments>,
    #[serde(default)]
    referenced_tweets: Option<Vec<ReferencedTweet>>,
}

impl Tweet {
    /// 回复链中的直接父帖 id（`referenced_tweets` 中 `type=replied_to`）。
    fn reply_to_id(&self) -> Option<&str> {
        let refs = self.referenced_tweets.as_ref()?;
        refs.iter()
            .find(|r| r.ref_type.as_str() == "replied_to")
            .map(|r| r.id.as_str())
    }
}

#[derive(Deserialize)]
struct ReferencedTweet {
    #[serde(rename = "type")]
    ref_type: String,
    id: String,
}

#[derive(Deserialize)]
struct Attachments {
    /// 无媒体推文或部分响应形态下可能省略；仅含 poll 等时也可能无此字段。
    #[serde(default)]
    media_keys: Vec<String>,
}

#[derive(Deserialize)]
struct MediaObj {
    media_key: String,
    #[serde(rename = "type")]
    media_type: String,
    url: Option<String>,
    variants: Option<Vec<Variant>>,
}

#[derive(Deserialize)]
struct Variant {
    content_type: Option<String>,
    bitrate: Option<u64>,
    url: Option<String>,
}
