//! POST /api/auth/login — 前端访问密码校验。
//!
//! body: {"password":"..."}，比对 state.frontend_password。
//! 成功返回 200 {"ok":true}，失败返回 401 {"error":"invalid password"}。
//! 本端点无鉴权（登录端点本身不能要求已登录）。

use crate::handlers::device_api::OkResponse;
use crate::handlers::{bad_request, json_response, AppResponse};
use crate::state::AppState;
use actix_http::StatusCode;
use bytes::Bytes;
use serde::Deserialize;
use subtle::ConstantTimeEq;

/// 登录请求体
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

/// 处理 POST /api/auth/login
pub async fn handle_login(state: &AppState, body: &Bytes, accept_gzip: bool) -> AppResponse {
    let req: LoginRequest = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return bad_request(&format!("无效 JSON: {e}"), accept_gzip),
    };
    let provided = req.password.as_bytes();
    let expected = state.frontend_password.as_bytes();
    let equal = bool::from(ConstantTimeEq::ct_eq(provided, expected));
    if equal {
        json_response(StatusCode::OK, &OkResponse { ok: true }, accept_gzip)
    } else {
        json_response(
            StatusCode::UNAUTHORIZED,
            &serde_json::json!({"error": "invalid password"}),
            accept_gzip,
        )
    }
}
