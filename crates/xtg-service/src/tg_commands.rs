//! 群内 @本账号 +「最新」：拉取配置中各 X 账号的最近一条推文并回复到当前会话。

use grammers_client::client::{UpdateStream, UpdatesConfiguration};
use grammers_client::update::Update;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, warn};
use xtg_core::Post;
use xtg_media::{download_post_media, temp_dir_for_post};
use xtg_tg_bridge::{GrammersSink, UpdatesLike};

use crate::command_state::CommandState;
use crate::translate::translate_to_zh;
use xtg_x_session::build_x_source;

fn biggest_post(posts: Vec<Post>) -> Option<Post> {
    posts.into_iter().max_by(|a, b| {
        let na: u128 = a.id.parse().unwrap_or(0);
        let nb: u128 = b.id.parse().unwrap_or(0);
        na.cmp(&nb)
    })
}

pub async fn run_update_listener(
    client: grammers_client::Client,
    mut stream: UpdateStream,
    state: Arc<std::sync::Mutex<CommandState>>,
    http: reqwest::Client,
    temp_base: PathBuf,
    log: broadcast::Sender<String>,
) {
    loop {
        let update = match stream.next().await {
            Ok(u) => u,
            Err(e) => {
                warn!("updates stream error: {e:?}");
                break;
            }
        };
        let Update::NewMessage(msg) = update else {
            continue;
        };
        if msg.outgoing() {
            continue;
        }
        if !msg.mentioned() {
            continue;
        }
        let t = msg.text();
        if !t.contains("最新") {
            continue;
        }

        let peer_ref = match msg.peer_ref().await {
            Some(p) => p,
            None => {
                let _ = log.send("无法解析群内会话，跳过「最新」".into());
                continue;
            }
        };

        let st = match state.lock() {
            Ok(g) => g.clone(),
            Err(_) => continue,
        };
        if st.handles.is_empty() {
            let _ = log.send("「最新」：未配置 X 账号".into());
            continue;
        }

        let sink = GrammersSink::new(client.clone());
        let max_mb = st.max_media_bytes;

        for h in &st.handles {
            let x = build_x_source(
                st.use_fake_x,
                &st.x_bearer_token,
                &st.x_api_base,
            );
            let posts = match x.fetch_latest(h, None).await {
                Ok(p) => p,
                Err(e) => {
                    let _ = log.send(format!("「最新」抓取 @{h} 失败: {e}"));
                    continue;
                }
            };

            let Some(post) = biggest_post(posts) else {
                let _ = log.send(format!("「最新」@{h} 无可用推文").into());
                continue;
            };

            let zh = if st.ai.enabled && !st.ai.api_key.trim().is_empty() {
                match translate_to_zh(&http, &st.ai, &post.text).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        let _ = log.send(format!("「最新」汉化失败: {e}"));
                        None
                    }
                }
            } else {
                None
            };

            let dir = temp_dir_for_post(&temp_base, &format!("latest_{}_{}", h, post.id));
            let files = match download_post_media(&http, &dir, &post, max_mb).await {
                Ok(f) => f,
                Err(e) => {
                    let _ = log.send(format!("「最新」下载媒体失败: {e}"));
                    vec![]
                }
            };

            let tr = zh.as_deref();
            if let Err(e) = sink
                .send_post_to_peer_ref(peer_ref, &post, &files, tr)
                .await
            {
                let _ = log.send(format!("「最新」发送失败: {e}"));
                error!("latest cmd send: {e}");
            } else {
                let _ = log.send(format!("「最新」已回复 @{h} 推文 {}", post.id));
            }
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    }
}

/// 由 `XtgService` 在连接 TG 后 `tokio::spawn` 一次；`updates` 只能消费一次。
pub async fn run_updates_task(
    client: grammers_client::Client,
    updates: tokio::sync::mpsc::UnboundedReceiver<UpdatesLike>,
    state: Arc<std::sync::Mutex<CommandState>>,
    http: reqwest::Client,
    temp_base: PathBuf,
    log: broadcast::Sender<String>,
) {
    let stream = client
        .stream_updates(updates, UpdatesConfiguration::default())
        .await;
    run_update_listener(client, stream, state, http, temp_base, log).await;
}
