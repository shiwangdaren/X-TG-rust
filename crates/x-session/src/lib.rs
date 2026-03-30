//! Twitter / X API v2（Bearer）拉取时间线；可选假数据用于 pipeline 测试。

mod api;

pub use api::TwitterApiV2Source;

use async_trait::async_trait;
use std::sync::Arc;
use xtg_core::{Post, Result, XSource};

/// 假数据，用于无 API 环境下的 pipeline 测试。
pub struct FakeXSource {
    pub posts: Vec<Post>,
}

#[async_trait]
impl XSource for FakeXSource {
    async fn fetch_latest(&self, _handle: &str, _since_id: Option<&str>) -> Result<Vec<Post>> {
        Ok(self.posts.clone())
    }
}

impl FakeXSource {
    pub fn sample() -> Self {
        Self {
            posts: vec![Post {
                id: "1234567890123456789".into(),
                author_handle: "sample".into(),
                text: "[fake] hello from xtg pipeline".into(),
                media: vec![],
                url: Some("https://x.com/sample/status/1234567890123456789".into()),
                posted_at_ms: None,
                reply_to_url: None,
            }],
        }
    }
}

/// 根据配置构造 X 数据源：假数据 → Twitter API v2（Bearer）。
pub fn build_x_source(
    use_fake_x: bool,
    x_bearer_token: &str,
    x_api_base: &str,
) -> Arc<dyn XSource> {
    if use_fake_x {
        return Arc::new(FakeXSource::sample());
    }
    Arc::new(TwitterApiV2Source::new(
        x_bearer_token.trim().to_string(),
        if x_api_base.trim().is_empty() {
            None
        } else {
            Some(x_api_base.trim().to_string())
        },
    ))
}
