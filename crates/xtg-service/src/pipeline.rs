use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tracing::{error, info};
use xtg_core::{tweet_id_to_unix_ms, JobConfig, Post, TgSink};
use xtg_media::{download_post_media, temp_dir_for_post};
use xtg_tg_bridge::GrammersSink;

use crate::state::TweetStore;
use crate::translate::translate_to_zh;

fn bigger_id(a: &str, b: &str) -> bool {
    let na: u128 = a.parse().unwrap_or(0);
    let nb: u128 = b.parse().unwrap_or(0);
    na > nb
}

fn log_line(tx: &broadcast::Sender<String>, line: String) {
    let _ = tx.send(line);
}

/// 单次轮询：对每个 handle 拉取、过滤新帖、下载媒体、发送。
pub async fn run_poll_round(
    x: &dyn xtg_core::XSource,
    client: &grammers_client::Client,
    http: &reqwest::Client,
    store_path: &Path,
    job: &JobConfig,
    temp_base: &Path,
    log: &broadcast::Sender<String>,
) {
    let handles = job.x_handles.clone();
    if handles.is_empty() {
        log_line(log, "未配置 X 账号".into());
        return;
    }
    if job.tg_targets.is_empty() {
        log_line(log, "未配置 Telegram 目标".into());
        return;
    }

    let tg = GrammersSink::new(client.clone());

    for h in handles {
        let last = match TweetStore::open(store_path) {
            Ok(s) => match s.last_id(&h) {
                Ok(v) => v,
                Err(e) => {
                    log_line(log, format!("读取游标失败: {e}"));
                    continue;
                }
            },
            Err(e) => {
                log_line(log, format!("打开游标文件失败: {e}"));
                continue;
            }
        };

        let posts = match x.fetch_latest(&h).await {
            Ok(p) => p,
            Err(e) => {
                log_line(log, format!("抓取 @{h} 失败: {e}"));
                error!("fetch {h}: {e}");
                continue;
            }
        };

        let poll_ms = job.poll_started_at_ms;
        let mut new_posts: Vec<Post> = posts
            .into_iter()
            .filter(|p| {
                let after_cursor = last
                    .as_ref()
                    .map(|lid| bigger_id(&p.id, lid))
                    .unwrap_or(true);
                if !after_cursor {
                    return false;
                }
                if poll_ms > 0 {
                    if let Some(tw) = tweet_id_to_unix_ms(&p.id) {
                        if tw < poll_ms {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();

        new_posts.sort_by(|a, b| a.id.cmp(&b.id));

        for post in new_posts {
            let zh_opt = if job.ai.enabled && !job.ai.api_key.trim().is_empty() {
                match translate_to_zh(http, &job.ai, &post.text).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        log_line(log, format!("汉化失败 {}: {e}", post.id));
                        None
                    }
                }
            } else {
                None
            };

            let dir = temp_dir_for_post(temp_base, &post.id);
            let max_b = job.max_media_bytes;
            let files = match download_post_media(http, &dir, &post, max_b).await {
                Ok(f) => f,
                Err(e) => {
                    log_line(log, format!("下载媒体失败 {}: {e}", post.id));
                    continue;
                }
            };

            let mut all_ok = true;
            for target in &job.tg_targets {
                match tg
                    .send_post(target, &post, &files, zh_opt.as_deref())
                    .await
                {
                    Ok(()) => {
                        log_line(log, format!("已发送 @{} 推文 {} → {}", h, post.id, target));
                        info!("sent {} to {}", post.id, target);
                    }
                    Err(e) => {
                        all_ok = false;
                        log_line(log, format!("TG 发送失败 {} → {}: {e}", post.id, target));
                    }
                }
            }
            if all_ok {
                if let Ok(s) = TweetStore::open(store_path) {
                    if let Err(e) = s.set_last_id(&h, &post.id) {
                        log_line(log, format!("保存游标失败: {e}"));
                    }
                }
            }

            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    }
}

pub fn default_store_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("cursors.json")
}
