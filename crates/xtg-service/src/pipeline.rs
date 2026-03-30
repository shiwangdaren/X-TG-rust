use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info};
use xtg_core::{tweet_id_to_unix_ms, JobConfig, Post, Result as CoreResult, TgSink};
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

/// 多账号时，相邻 handle 之间错开间隔（秒），避免同一轮内同时打 X API。
fn stagger_between_handles(interval_secs: f64, n_handles: usize) -> Duration {
    if n_handles <= 1 {
        return Duration::ZERO;
    }
    let base = (interval_secs / n_handles as f64).clamp(0.5, 5.0);
    Duration::from_secs_f64(base)
}

const FETCH_RETRY_MAX: u32 = 5;
const BACKOFF_INITIAL_MS: u64 = 1000;
const BACKOFF_MAX_MS: u64 = 60_000;

async fn fetch_latest_with_backoff(
    x: &dyn xtg_core::XSource,
    handle: &str,
    since_id: Option<&str>,
    log: &broadcast::Sender<String>,
) -> CoreResult<Vec<Post>> {
    let mut delay_ms = BACKOFF_INITIAL_MS;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match x.fetch_latest(handle, since_id).await {
            Ok(p) => return Ok(p),
            Err(e) if attempt < FETCH_RETRY_MAX => {
                log_line(
                    log,
                    format!(
                        "@{handle} 抓取失败 ({}/{}): {e}，{}ms 后重试",
                        attempt, FETCH_RETRY_MAX, delay_ms
                    ),
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                delay_ms = (delay_ms.saturating_mul(2)).min(BACKOFF_MAX_MS);
            }
            Err(e) => return Err(e),
        }
    }
}

/// 单次轮询：对每个 handle 错开拉取、指数退避重试、过滤新帖、下载媒体、发送。
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

    let n = handles.len();
    let stagger = stagger_between_handles(job.poll_interval_secs, n);
    for (idx, h) in handles.into_iter().enumerate() {
        if idx > 0 && stagger > Duration::ZERO {
            tokio::time::sleep(stagger).await;
        }

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

        let since = last.as_deref();
        let posts = match fetch_latest_with_backoff(x, &h, since, log).await {
            Ok(p) => p,
            Err(e) => {
                log_line(log, format!("抓取 @{h} 最终失败: {e}"));
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
