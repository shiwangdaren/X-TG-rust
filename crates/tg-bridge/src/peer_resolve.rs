use grammers_client::{Client, InvocationError};
use grammers_client::peer::Peer;
use grammers_mtsender::RpcError;

/// 若 `target` 为纯数字（含负号），在对话列表中查找匹配的 Bot API dialog id；否则按 @username 解析。
pub async fn resolve_target_peer(client: &Client, target: &str) -> Result<Peer, InvocationError> {
    let t = target.trim();
    let u = t.trim_start_matches('@');

    if !u.is_empty() && u.chars().all(|c| c.is_ascii_digit() || c == '-') {
        if let Ok(id) = t.parse::<i64>() {
            return find_peer_by_bot_api_id(client, id).await;
        }
    }

    match client.resolve_username(u).await? {
        Some(p) => Ok(p),
        None => Err(InvocationError::Rpc(RpcError {
            code: 400,
            name: "USERNAME_NOT_RESOLVED".into(),
            value: None,
            caused_by: None,
        })),
    }
}

async fn find_peer_by_bot_api_id(
    client: &Client,
    bot_api_id: i64,
) -> Result<Peer, InvocationError> {
    let mut iter = client.iter_dialogs();
    while let Some(d) = iter.next().await? {
        if d.peer_id().bot_api_dialog_id() == bot_api_id {
            return Ok(d.peer.clone());
        }
    }
    Err(InvocationError::Rpc(RpcError {
        code: 400,
        name: "PEER_NOT_IN_DIALOGS".into(),
        value: None,
        caused_by: None,
    }))
}
