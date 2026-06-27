//! GET/POST /api/pwm_cache/{deviceId} — 查询或切换 PWM 缓存开关。
//!
//! PWM 缓存语义：设备本地维护方向→PWM 的默认值映射表，
//! 启用后控制指令可省略 pwm 字段（设备使用缓存值）。

use crate::device::OutboundCommand;
use crate::handlers::{bad_request, device_not_found, json_response, AppResponse};
use crate::protocol::DeviceCommand;
use crate::state::AppState;
use actix_http::StatusCode;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// GET 响应：{enabled, entries}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PwmCacheState {
    pub enabled: bool,
    pub entries: Vec<PwmCacheEntry>,
}

/// 缓存档位条目
#[derive(Debug, Serialize)]
pub struct PwmCacheEntry {
    pub speed: &'static str,
    pub pwm: u16,
}

/// POST 请求体：{enabled}
#[derive(Debug, Deserialize)]
pub struct PwmCacheToggleRequest {
    pub enabled: bool,
}

/// POST 响应：{ok: true}
#[derive(Debug, Serialize)]
pub struct PwmCacheToggleResponse {
    pub ok: bool,
}

/// 默认档位表（设备固件侧常量）
fn default_entries() -> Vec<PwmCacheEntry> {
    vec![
        PwmCacheEntry { speed: "low", pwm: 120 },
        PwmCacheEntry { speed: "mid", pwm: 180 },
        PwmCacheEntry { speed: "high", pwm: 255 },
    ]
}

/// 处理 GET /api/pwm_cache/{deviceId}
pub async fn handle_get(state: &AppState, device_id: &str, accept_gzip: bool) -> AppResponse {
    let entry = match state.registry.get(device_id) {
        Some(e) => e,
        None => return device_not_found(device_id, accept_gzip),
    };
    let state_resp = PwmCacheState {
        enabled: entry.pwm_cache_enabled(),
        entries: default_entries(),
    };
    json_response(StatusCode::OK, &state_resp, accept_gzip)
}

/// 处理 POST /api/pwm_cache/{deviceId}
pub async fn handle_post(
    state: &AppState,
    device_id: &str,
    body: &Bytes,
    accept_gzip: bool,
) -> AppResponse {
    let entry = match state.registry.get(device_id) {
        Some(e) => e,
        None => return device_not_found(device_id, accept_gzip),
    };
    let req: PwmCacheToggleRequest = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return bad_request(&format!("无效 JSON: {e}"), accept_gzip),
    };

    // 本地状态切换
    entry.set_pwm_cache(req.enabled);

    // 下发到设备
    let cmd = DeviceCommand::PwmCache {
        enabled: req.enabled,
        ts: crate::device::now_ms(),
    };
    if let Ok(json) = serde_json::to_vec(&cmd) {
        let outbound = OutboundCommand {
            device_id: device_id.to_string(),
            json: Bytes::from(json),
        };
        if state.cmd_sink.send(outbound).await.is_err() {
            log::warn!("出站指令通道已关闭（设备 {device_id} 的 pwm_cache 被丢弃）");
        }
    }

    json_response(
        StatusCode::OK,
        &PwmCacheToggleResponse { ok: true },
        accept_gzip,
    )
}
