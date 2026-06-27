//! POST /api/control/{deviceId} — 下发 WASD + PWM 控制指令。
//!
//! 流程：解析 JSON → 构造 DeviceCommand → 推入设备指令队列
//! → 设备 HTTPS 长轮询拉取并执行 → ack 通过 POST /api/device/{id}/event 回流。

use crate::handlers::{bad_request, device_not_found, json_response, AppResponse};
use crate::protocol::DeviceCommand;
use crate::state::AppState;
use actix_http::StatusCode;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// 请求体：{direction, pwm}
#[derive(Debug, Deserialize)]
pub struct ControlRequest {
    /// "W" | "A" | "S" | "D" | "stop"
    pub direction: String,
    /// 0..255
    pub pwm: u16,
}

/// 响应体：{ok: true}
#[derive(Debug, Serialize)]
pub struct ControlResponse {
    pub ok: bool,
}

/// 合法方向集合（用于服务端校验）
const VALID_DIRECTIONS: &[&str] = &["W", "A", "S", "D", "stop"];

/// 处理 POST /api/control/{deviceId}
pub async fn handle_post_control(
    state: &AppState,
    device_id: &str,
    body: &Bytes,
    accept_gzip: bool,
) -> AppResponse {
    // 设备存在性
    let entry = match state.registry.get(device_id) {
        Some(e) => e,
        None => return device_not_found(device_id, accept_gzip),
    };
    // 解析请求体
    let req: ControlRequest = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return bad_request(&format!("无效 JSON: {e}"), accept_gzip),
    };
    // 字段校验
    if !VALID_DIRECTIONS.contains(&req.direction.as_str()) {
        return bad_request(
            &format!("direction 必须为 {:?} 之一", VALID_DIRECTIONS),
            accept_gzip,
        );
    }
    if req.pwm > 255 {
        return bad_request("pwm 必须在 0..255 范围内", accept_gzip);
    }

    // 构造下行指令并推入队列（设备下次 poll 时取走）
    let cmd = DeviceCommand::Control {
        direction: req.direction.clone(),
        pwm: req.pwm,
        duration_ms: None,
        ts: crate::device::now_ms(),
    };
    let json = match serde_json::to_vec(&cmd) {
        Ok(v) => Bytes::from(v),
        Err(e) => {
            log::error!("序列化 control 指令失败: {e}");
            return bad_request("内部序列化错误", accept_gzip);
        }
    };
    if entry.try_push_command(json).is_err() {
        log::warn!("设备 {device_id} 指令队列已满，control 被丢弃");
    }

    json_response(StatusCode::OK, &ControlResponse { ok: true }, accept_gzip)
}
