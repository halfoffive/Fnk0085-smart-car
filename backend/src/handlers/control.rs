//! POST /api/control/{deviceId} — 下发 WASD + PWM 控制指令。
//!
//! 流程：解析 JSON → 构造 DeviceCommand → AEAD 加密后通过 UDP 发送 → 立即返回 ok。
//! 设备的 ack 通过单独的 UDP 事件回流，不在此 handler 等待。

use crate::device::OutboundCommand;
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
    if state.registry.get(device_id).is_none() {
        return device_not_found(device_id, accept_gzip);
    }
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

    // 构造下行指令
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

    // 投递至 UDP 出站循环（不等待实际发送完成，纯异步）
    let outbound = OutboundCommand {
        device_id: device_id.to_string(),
        json,
    };
    if state.cmd_sink.send(outbound).await.is_err() {
        log::warn!("出站指令通道已关闭（设备 {device_id} 的 control 被丢弃）");
    }

    json_response(StatusCode::OK, &ControlResponse { ok: true }, accept_gzip)
}
