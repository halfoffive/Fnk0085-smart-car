//! 设备侧 HTTPS 接口：register / poll / event。
//!
//! 替代原 UDP register + UDP JSON 事件 + UDP 出站指令：
//! - 设备启动后 POST /api/device/{id}/register 注册（token 校验）
//! - 设备长轮询 GET /api/device/{id}/poll?timeout=30 拉取指令
//! - 设备 POST /api/device/{id}/event 上报 photo_done / ack / error
//!
//! 视频流仍走 UDP + AEAD（高带宽低延时），与 HTTPS 控制通道分离。

use crate::device::{now_ms, PhotoResult};
use crate::handlers::{bad_request, device_not_found, json_response, AppResponse};
use crate::protocol::DeviceEvent;
use crate::state::AppState;
use actix_http::{Request, StatusCode};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// poll 默认与最大超时（秒）
const POLL_DEFAULT_TIMEOUT_S: u64 = 30;
const POLL_MAX_TIMEOUT_S: u64 = 60;

/// register 请求体：{token}
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub token: String,
}

/// 通用 ok 响应
#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

/// 处理 POST /api/device/{deviceId}/register
///
/// 校验 token，成功则创建/刷新设备项。
pub async fn handle_register(
    state: &AppState,
    device_id: &str,
    body: &Bytes,
    accept_gzip: bool,
) -> AppResponse {
    let req: RegisterRequest = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return bad_request(&format!("无效 JSON: {e}"), accept_gzip),
    };
    if !validate_token(&req.token, &state.expected_token) {
        return json_response(
            StatusCode::UNAUTHORIZED,
            &serde_json::json!({"error": "invalid token"}),
            accept_gzip,
        );
    }
    match state.registry.get_or_create(device_id) {
        Ok(entry) => {
            entry.touch(None, now_ms());
            log::info!("设备 {device_id} HTTPS 注册成功");
            json_response(StatusCode::OK, &OkResponse { ok: true }, accept_gzip)
        }
        Err(e) => {
            log::warn!("设备 {device_id} 注册失败: {e}");
            json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &serde_json::json!({"error": "registry full"}),
                accept_gzip,
            )
        }
    }
}

/// 处理 GET /api/device/{deviceId}/poll?timeout=N
///
/// 长轮询：等待指令入队，超时返回 Ping 占位包。
/// Authorization: Bearer <token> 必须匹配。
pub async fn handle_poll(
    state: &AppState,
    device_id: &str,
    req: &Request,
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
    let timeout_s = parse_timeout(req.head().uri.query());
    // 同一设备同一时刻仅允许一个 poll 持锁，防止指令重复消费
    let mut rx = entry.cmd_rx.lock().await;
    let cmd_json = match tokio::time::timeout(Duration::from_secs(timeout_s), rx.recv()).await {
        Ok(Some(json)) => json, // 拿到真实指令
        Ok(None) => {
            // 队列关闭（设备项被回收，理论上不会发生）
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &serde_json::json!({"error": "command channel closed"}),
                accept_gzip,
            );
        }
        Err(_) => {
            // 超时：返回 Ping 占位，让设备立即重新发起 poll
            let ping = crate::protocol::DeviceCommand::Ping {
                seq: 0,
                ts: now_ms(),
            };
            Bytes::from(serde_json::to_vec(&ping).unwrap_or_default())
        }
    };
    // 直接回吐指令 JSON 字节流
    let mut builder = actix_http::Response::build(StatusCode::OK);
    builder
        .content_type("application/json")
        .insert_header(("Cache-Control", "no-store"));
    builder.body(cmd_json).map_into_boxed_body()
}

/// 处理 POST /api/device/{deviceId}/event
///
/// 接收设备上报的 photo_done / ack / error 事件。
/// Authorization: Bearer <token> 必须匹配。
pub async fn handle_event(
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
    let body = match crate::handlers::read_body(req).await {
        Ok(b) => b,
        Err(e) => return bad_request(&format!("读取请求体失败: {e}"), accept_gzip),
    };
    let event: DeviceEvent = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return bad_request(&format!("无效 JSON: {e}"), accept_gzip),
    };
    match event {
        DeviceEvent::Register { .. } => {
            // register 应走 /register 端点；此处拒绝避免误用
            return bad_request(
                "register 应通过 POST /api/device/{id}/register",
                accept_gzip,
            );
        }
        DeviceEvent::PhotoDone { path, uptime_ms: _ } => {
            let ok = entry.complete_photo(PhotoResult { path: path.clone() });
            if ok {
                log::info!("设备 {device_id} 拍照完成: {path}");
            } else {
                log::debug!("设备 {device_id} 收到 photo_done 但无挂起请求");
            }
        }
        DeviceEvent::Ack { ref_seq } => {
            log::debug!("设备 {device_id} ack refSeq={ref_seq}");
        }
        DeviceEvent::Error { code, message } => {
            log::warn!("设备 {device_id} 错误 code={code} message={message}");
        }
    }
    entry.touch(None, now_ms());
    json_response(StatusCode::OK, &OkResponse { ok: true }, accept_gzip)
}

/// 校验 Authorization: Bearer <token> 头
fn check_auth(req: &Request, expected: &str) -> bool {
    let header = match req
        .head()
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        Some(h) => h,
        None => return false,
    };
    let received = header.strip_prefix("Bearer ").unwrap_or(header);
    received == expected
}

/// 解析 query string 中的 timeout=N（秒），缺省 30，上限 60
fn parse_timeout(query: Option<&str>) -> u64 {
    let q = match query {
        Some(s) => s,
        None => return POLL_DEFAULT_TIMEOUT_S,
    };
    for pair in q.split('&') {
        if let Some(rest) = pair.strip_prefix("timeout=") {
            if let Ok(n) = rest.parse::<u64>() {
                return n.clamp(1, POLL_MAX_TIMEOUT_S);
            }
        }
    }
    POLL_DEFAULT_TIMEOUT_S
}

/// 校验 register token：与配置 token 比对，允许 "Bearer " 前缀
fn validate_token(received: &str, expected: &str) -> bool {
    let trimmed = received.strip_prefix("Bearer ").unwrap_or(received);
    trimmed == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation() {
        assert!(validate_token("change-me-please", "change-me-please"));
        assert!(validate_token(
            "Bearer change-me-please",
            "change-me-please",
        ));
        assert!(!validate_token("wrong", "change-me-please"));
    }

    #[test]
    fn timeout_parsing() {
        assert_eq!(parse_timeout(None), POLL_DEFAULT_TIMEOUT_S);
        assert_eq!(parse_timeout(Some("timeout=10")), 10);
        assert_eq!(parse_timeout(Some("timeout=999")), POLL_MAX_TIMEOUT_S);
        assert_eq!(parse_timeout(Some("foo=bar")), POLL_DEFAULT_TIMEOUT_S);
        assert_eq!(parse_timeout(Some("timeout=abc")), POLL_DEFAULT_TIMEOUT_S);
    }
}
