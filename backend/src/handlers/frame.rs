//! POST /api/device/{deviceId}/frame — 设备上传单帧 JPEG。
//!
//! 流程：check_auth Bearer token → registry.get(device_id) → read_body
//! → entry.video.push(Frame { server_recv_ms: now_ms(), jpeg: body })
//! → 返回 204 No Content。
//!
//! 设计：
//! - 失败帧由设备端丢弃（不重试），后端不维护重试状态。
//! - 帧的 server_recv_ms 用于 multipart 响应 X-Latency-Ms 计算。

use crate::device::now_ms;
use crate::handlers::device_api::check_auth;
use crate::handlers::{bad_request, device_not_found, json_response, read_body, AppResponse};
use crate::state::AppState;
use crate::video_cache::Frame;
use actix_http::{Request, StatusCode};
use bytes::Bytes;

/// 处理 POST /api/device/{deviceId}/frame
///
/// 设备以 `Content-Type: image/jpeg` + `Authorization: Bearer <token>` +
/// `X-Device-Uptime-Ms: <ms>` 头 POST 单帧 JPEG 二进制 body。
/// 后端校验 token 后将 JPEG push 进 VideoCache，返回 204。
pub async fn handle_frame(
    state: &AppState,
    device_id: &str,
    req: &mut Request,
    accept_gzip: bool,
) -> AppResponse {
    if !check_auth(req, &state.expected_token) {
        return json_response(
            StatusCode::UNAUTHORIZED,
            &serde_json::json!({"error": "unauthorized"}),
            accept_gzip,
        );
    }
    let entry = match state.registry.get(device_id) {
        Some(e) => e,
        None => return device_not_found(device_id, accept_gzip),
    };
    let body: Bytes = match read_body(req).await {
        Ok(b) => b,
        Err(e) => return bad_request(&format!("读取请求体失败: {e}"), accept_gzip),
    };
    // 视为心跳：10fps 帧上传会持续刷新 last_seen，使设备保持 online
    let recv_ms = now_ms();
    entry.touch(None, recv_ms);
    entry.video.push(Frame {
        server_recv_ms: recv_ms,
        jpeg: body,
    });
    // 204 No Content：设备仅关心状态码，不需要 body
    actix_http::Response::build(StatusCode::NO_CONTENT)
        .body(Bytes::new())
        .map_into_boxed_body()
}
