//! POST /api/photo/{deviceId} — 触发设备拍照并等待 photo_done 回执。
//!
//! 流程：
//! 1. 在 DeviceEntry 上注册 oneshot 等待器。
//! 2. 向设备指令队列推入 photo 指令（设备 HTTPS 长轮询拉取）。
//! 3. 等待 oneshot 回执或 8s 超时。
//! 4. 设备拍照完成后 POST /api/device/{id}/event 上报 photo_done → 触发 oneshot。

use crate::device::PhotoResult;
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

    // 推入 photo 指令
    let cmd = DeviceCommand::Photo {
        quality: "max".to_string(),
        ts: crate::device::now_ms(),
    };
    if let Ok(json) = serde_json::to_vec(&cmd) {
        if entry.try_push_command(Bytes::from(json)).is_err() {
            log::warn!("设备 {device_id} 指令队列已满，photo 被丢弃");
        }
    }

    // 等待回执或超时
    match tokio::time::timeout(Duration::from_millis(PHOTO_TIMEOUT_MS), rx).await {
        Ok(Ok(result)) if result.ok => json_response(
            StatusCode::OK,
            &PhotoResponse { path: result.path },
            accept_gzip,
        ),
        Ok(Ok(_)) => {
            // 设备上报 error 事件触发的释放（ok=false）→ 设备侧失败，返回 502 区别于 504 超时
            json_response(
                StatusCode::BAD_GATEWAY,
                &serde_json::json!({"error": "device capture failed"}),
                accept_gzip,
            )
        }
        Ok(Err(_)) => {
            // oneshot 被 drop（不应发生，但防御性处理）
            photo_timeout(accept_gzip)
        }
        Err(_) => {
            // 超时：清理挂起的等待器
            entry.complete_photo(PhotoResult {
                path: String::new(),
                ok: false,
            });
            photo_timeout(accept_gzip)
        }
    }
}
