//! GET /api/devices — 返回当前注册设备的在线状态摘要。
//!
//! 纯查询 handler，无副作用。

use crate::handlers::{json_response, AppResponse};
use crate::state::AppState;
use actix_http::StatusCode;

/// 处理 GET /api/devices
pub async fn handle_get_devices(state: &AppState, accept_gzip: bool) -> AppResponse {
    let devices = state.registry.list();
    // protocol.md §4.1：返回数组 [{deviceId, online, lastSeenMs}]
    json_response(StatusCode::OK, &devices, accept_gzip)
}
