//! HTTP 下载推文媒体到临时目录。

use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};
use xtg_core::{CoreError, DownloadedMedia, MediaKind, Post, Result};

fn guess_ext(kind: MediaKind, url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains(".mp4") || lower.contains("video") {
        return "mp4";
    }
    if lower.contains(".gif") {
        return "gif";
    }
    match kind {
        MediaKind::Image => "jpg",
        MediaKind::Video => "mp4",
        MediaKind::Gif => "gif",
        MediaKind::Unknown => "bin",
    }
}

fn guess_mime(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
    .to_string())
}

/// 将帖子中的远程媒体下载到 `dir`（通常为系统临时目录子路径）。
pub async fn download_post_media(
    client: &reqwest::Client,
    dir: &Path,
    post: &Post,
    max_bytes: u64,
) -> Result<Vec<DownloadedMedia>> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| CoreError::Fetch(e.to_string()))?;

    let mut out = Vec::new();
    for (i, m) in post.media.iter().enumerate() {
        let ext = guess_ext(m.kind, &m.url);
        let name = format!("{}_{}.{}", post.id, i, ext);
        let path = dir.join(name);

        match download_one(client, &m.url, &path, max_bytes).await {
            Ok(()) => {
                let mime = guess_mime(&path);
                out.push(DownloadedMedia { path, mime });
            }
            Err(e) => {
                warn!("skip media {}: {}", m.url, e);
            }
        }
    }
    Ok(out)
}

async fn download_one(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> std::result::Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let len = resp.content_length().unwrap_or(0);
    if len > max_bytes {
        return Err(format!("content-length {} exceeds cap {}", len, max_bytes));
    }

    let mut stream = resp.bytes_stream();
    let mut file = File::create(dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        total += chunk.len() as u64;
        if total > max_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err("download exceeded max_bytes".into());
        }
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    debug!("downloaded {} -> {}", url, dest.display());
    Ok(())
}

/// 为单次帖子创建临时目录。
pub fn temp_dir_for_post(base: &Path, post_id: &str) -> PathBuf {
    base.join(format!("xtg-{}", post_id))
}
