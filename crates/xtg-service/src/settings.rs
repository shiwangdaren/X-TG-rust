use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use xtg_core::AiJobConfig;

fn default_x_api_base() -> String {
    "https://api.twitter.com".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub api_id: String,
    pub api_hash: String,
    pub phone: String,
    pub tg_session_path: String,
    /// 每行一个：Telegram 群组/频道 @username 或数字 id；轮询帖会发往每一行对应目标。
    pub tg_targets: String,
    pub x_handles: String,
    pub poll_interval_secs: f64,
    pub max_media_mb: u64,
    pub use_fake_x: bool,
    /// OAuth 2.0 App-only Bearer Token（来自 X Developer Portal）。
    #[serde(default)]
    pub x_bearer_token: String,
    /// API 根地址，留空为 `https://api.twitter.com`。
    #[serde(default = "default_x_api_base")]
    pub x_api_base: String,
    #[serde(default)]
    pub ai_enabled: bool,
    #[serde(default)]
    pub ai_api_base: String,
    #[serde(default)]
    pub ai_api_key: String,
    #[serde(default)]
    pub ai_model: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let base = paths::data_dir();
        Self {
            api_id: String::new(),
            api_hash: String::new(),
            phone: String::new(),
            tg_session_path: base.join("telegram.session").to_string_lossy().into(),
            tg_targets: String::new(),
            x_handles: String::new(),
            poll_interval_secs: 60.0,
            max_media_mb: 50,
            use_fake_x: true,
            x_bearer_token: String::new(),
            x_api_base: default_x_api_base(),
            ai_enabled: false,
            ai_api_base: "https://api.x.ai/v1".into(),
            ai_api_key: String::new(),
            ai_model: "grok-3-mini".into(),
        }
    }
}

impl AppSettings {
    pub fn ai_job_config(&self) -> AiJobConfig {
        let trimmed_base = self.ai_api_base.trim();
        let api_base = if trimmed_base.is_empty() {
            "https://api.x.ai/v1".into()
        } else {
            let mut b = trimmed_base.to_string();
            if b == "https://api.openai.com/v1" {
                b = "https://api.x.ai/v1".into();
            }
            b
        };
        let model = if !self.ai_model.trim().is_empty() {
            self.ai_model.clone()
        } else {
            "grok-3-mini".into()
        };
        AiJobConfig {
            enabled: self.ai_enabled,
            api_base,
            api_key: self.ai_api_key.clone(),
            model,
        }
    }

    /// 解析 TG 目标行（去空行、首尾空白）。
    pub fn tg_target_list(&self) -> Vec<String> {
        self.tg_targets
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let s = std::fs::read_to_string(path)?;
            let mut val: toml::Value = toml::from_str(&s)?;
            if let Some(m) = val.as_table_mut() {
                m.remove("ai_provider");
                if let Some(old) = m.remove("tg_target") {
                    if !m.contains_key("tg_targets") {
                        m.insert("tg_targets".into(), old);
                    }
                }
            }
            let s = toml::to_string(&val)?;
            let mut v: AppSettings = toml::from_str(&s)?;
            v.poll_interval_secs = v.poll_interval_secs.clamp(0.1, 3600.0);
            Ok(v)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut copy = self.clone();
        copy.poll_interval_secs = copy.poll_interval_secs.clamp(0.1, 3600.0);
        std::fs::write(path, toml::to_string_pretty(&copy)?)?;
        Ok(())
    }

    pub fn tg_session_path_buf(&self) -> PathBuf {
        PathBuf::from(&self.tg_session_path)
    }
}
