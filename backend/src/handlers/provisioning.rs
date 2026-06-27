//! GET /api/config — 返回当前服务器地址与 token，供 Web Serial 配网弹窗自动填充。
//!
//! 该端点无需鉴权：前端在配网前尚未与固件建立关联，且 token 需要暴露给固件。

use crate::handlers::{json_response, AppResponse};
use crate::state::AppState;
use actix_http::StatusCode;
use serde::Serialize;

/// 配网配置响应体（camelCase，与前端表单字段一致）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResponse {
    pub server: String,
    pub token: String,
}

/// 处理 GET /api/config
///
/// 返回本服务监听地址（host:port）与配置的 token。
pub async fn handle_get_config(state: &AppState, accept_gzip: bool) -> AppResponse {
    json_response(
        StatusCode::OK,
        &ConfigResponse {
            server: state.server_addr.clone(),
            token: (*state.expected_token).clone(),
        },
        accept_gzip,
    )
}
