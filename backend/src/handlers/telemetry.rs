//! GET /api/telemetry/{deviceId} — 返回设备左右轮速。
//!
//! 设备通过 POST /api/device/{id}/event 上报 `type=telemetry` 事件后，
//! 后端将左右轮速缓存在 `DeviceEntry` 中，本端点供前端查询。

use crate::handlers::{device_not_found, json_response, AppResponse};
use crate::state::AppState;
use actix_http::StatusCode;
use serde::Serialize;

/// 轮速响应体（camelCase，与 protocol.md 保持一致）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryResponse {
    pub left_rpm: i32,
    pub right_rpm: i32,
}

/// 处理 GET /api/telemetry/{deviceId}
///
/// 设备不在线时返回 404，避免前端展示过期数据。
pub async fn handle_get(state: &AppState, device_id: &str, accept_gzip: bool) -> AppResponse {
    let entry = match state.registry.get(device_id) {
        Some(e) if e.is_online() => e,
        _ => return device_not_found(device_id, accept_gzip),
    };
    let (left_rpm, right_rpm) = entry.telemetry();
    json_response(
        StatusCode::OK,
        &TelemetryResponse {
            left_rpm,
            right_rpm,
        },
        accept_gzip,
    )
}
