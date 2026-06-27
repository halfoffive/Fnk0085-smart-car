//! POST /api/photo/{deviceId} — 触发设备拍照并等待 photo_done 回执。
//!
//! 流程：
//! 1. 在 DeviceEntry 上注册 oneshot 等待器。
//! 2. 向设备下发 photo 指令（UDP AEAD）。
//! 3. 等待 oneshot 回执或 8s 超时。
//! 4. 收到回执后返回 {path}。

use crate::device::{OutboundCommand, PhotoResult};
use crate::handlers::{device_not_found, json_response, photo_timeout, AppResponse};
use crate::protocol::DeviceCommand;
use crate::state::AppState;
use actix_http::StatusCode;
use bytes::Bytes;
use serde::Serialize;
use std::time::Duration;
use tokio::sync::oneshot;

/// 响应体：{path}
#[derive(Debug, Serialize)]
pub struct PhotoResponse {
    pub path: String,
}

/// 拍照等待超时（ms）
const PHOTO_TIMEOUT_MS: u64 = 8_000;

/// 处理 POST /api/photo/{deviceId}
pub async fn handle_post_photo(
    state: &AppState,
    device_id: &str,
    accept_gzip: bool,
) -> AppResponse {
    let entry = match state.registry.get(device_id) {
        Some(e) => e,
        None => return device_not_found(device_id, accept_gzip),
    };

    // 注册 oneshot 等待器
    let (tx, rx) = oneshot::channel::<PhotoResult>();
    if !entry.await_photo(tx) {
        return crate::handlers::bad_request("已有拍照请求挂起，请等待完成", accept_gzip);
    }

    // 下发 photo 指令
    let cmd = DeviceCommand::Photo {
        quality: "max".to_string(),
        ts: crate::device::now_ms(),
    };
    let json = match serde_json::to_vec(&cmd) {
        Ok(v) => Bytes::from(v),
        Err(e) => {
            log::error!("序列化 photo 指令失败: {e}");
            return crate::handlers::bad_request("内部序列化错误", accept_gzip);
        }
    };
    let outbound = OutboundCommand {
        device_id: device_id.to_string(),
        json,
    };
    if state.cmd_sink.send(outbound).await.is_err() {
        log::warn!("出站指令通道已关闭（设备 {device_id} 的 photo 被丢弃）");
    }

    // 等待回执或超时
    match tokio::time::timeout(Duration::from_millis(PHOTO_TIMEOUT_MS), rx).await {
        Ok(Ok(result)) => json_response(
            StatusCode::OK,
            &PhotoResponse { path: result.path },
            accept_gzip,
        ),
        Ok(Err(_)) => {
            // oneshot 被 drop（不应发生，但防御性处理）
            photo_timeout(accept_gzip)
        }
        Err(_) => {
            // 超时：清理挂起的等待器
            entry.complete_photo(PhotoResult {
                path: String::new(),
            });
            photo_timeout(accept_gzip)
        }
    }
}
