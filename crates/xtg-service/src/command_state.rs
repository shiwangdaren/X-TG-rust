//! 供群内「最新」指令与轮询任务读取的 X / AI 配置快照。

use crate::settings::AppSettings;

#[derive(Clone)]
pub struct CommandState {
    pub handles: Vec<String>,
    pub use_fake_x: bool,
    pub x_bearer_token: String,
    pub x_api_base: String,
    pub ai: xtg_core::AiJobConfig,
    pub max_media_bytes: u64,
}

impl CommandState {
    pub fn from_settings(s: &AppSettings) -> Self {
        let handles: Vec<String> = s
            .x_handles
            .lines()
            .map(|l| l.trim().trim_start_matches('@').to_string())
            .filter(|x| !x.is_empty())
            .collect();
        Self {
            handles,
            use_fake_x: s.use_fake_x,
            x_bearer_token: s.x_bearer_token.clone(),
            x_api_base: s.x_api_base.clone(),
            ai: s.ai_job_config(),
            max_media_bytes: s.max_media_mb * 1024 * 1024,
        }
    }
}
