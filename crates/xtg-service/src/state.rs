//! 推文游标：每个 X 账号最后已转发的推文 id（JSON 文件，避免与 grammers/libsql 重复链接 SQLite）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use xtg_core::CoreError;

#[derive(Default, Serialize, Deserialize)]
struct CursorFile {
    #[serde(flatten)]
    cursors: HashMap<String, String>,
}

pub struct TweetStore {
    path: PathBuf,
}

impl TweetStore {
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| CoreError::Storage(e.to_string()))?;
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn load(&self) -> Result<CursorFile, CoreError> {
        if !self.path.exists() {
            return Ok(CursorFile::default());
        }
        let s = std::fs::read_to_string(&self.path)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        serde_json::from_str(&s).map_err(|e| CoreError::Storage(e.to_string()))
    }

    fn save(&self, data: &CursorFile) -> Result<(), CoreError> {
        let s = serde_json::to_string_pretty(data).map_err(|e| CoreError::Storage(e.to_string()))?;
        std::fs::write(&self.path, s).map_err(|e| CoreError::Storage(e.to_string()))
    }

    pub fn last_id(&self, handle: &str) -> Result<Option<String>, CoreError> {
        let data = self.load()?;
        Ok(data.cursors.get(handle).cloned())
    }

    pub fn set_last_id(&self, handle: &str, tweet_id: &str) -> Result<(), CoreError> {
        let mut data = self.load()?;
        data.cursors.insert(handle.to_string(), tweet_id.to_string());
        self.save(&data)
    }
}
