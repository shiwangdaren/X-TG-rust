//! 数据目录：优先环境变量 `XTG_DATA_DIR`（适合 Ubuntu `/var/lib/xtg`），否则 XDG / 各平台默认。

use std::path::PathBuf;

/// 应用数据根目录（已尝试 `create_dir_all`）。
pub fn data_dir() -> PathBuf {
    let dir = if let Ok(p) = std::env::var("XTG_DATA_DIR") {
        PathBuf::from(p.trim())
    } else {
        directories::ProjectDirs::from("com", "xtg", "xtg-app")
            .map(|p| p.data_dir().to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir().join("xtg-app"))
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}
