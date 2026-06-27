//! HTTP handlers：路由分发与各 API 处理器。
//!
//! 路由策略：基于 path + method 的简单前缀匹配（避免引入路由框架以控制内存）。
//! gzip 中间件：对返回 Bytes 体的 handler，传入 `accept_gzip` 标志，
//! 在构造响应时直接压缩并设置 Content-Encoding。视频流（BodyStream）不压缩。

pub mod control;
pub mod device_api;
pub mod devices;
pub mod frame;
pub mod photo;
pub mod pwm_cache;
pub mod stream;

use crate::state::AppState;
use actix_http::body::BoxBody;
use actix_http::{Method, Request, Response, StatusCode};
use bytes::Bytes;
use futures::StreamExt;
use std::convert::Infallible;

/// 统一响应类型：所有 handler 返回 `Response<BoxBody>`。
pub type AppResponse = Response<BoxBody>;

/// 主路由：根据 path + method 分发到各 handler。
pub async fn route(req: Request, state: AppState) -> Result<AppResponse, Infallible> {
    let head = req.head();
    let method = head.method.clone();
    let path = head.uri.path().to_string();
    let accept_gzip = accepts_gzip(
        head.headers
            .get("accept-encoding")
            .and_then(|v| v.to_str().ok()),
    );

    if let Some(resp) = try_route_api(&method, &path, req, &state, accept_gzip).await {
        return Ok(resp);
    }
    // 静态资源
    if method == Method::GET {
        if let Some(resp) = try_serve_static(&path, accept_gzip) {
            return Ok(resp);
        }
    }
    Ok(not_found(accept_gzip))
}

/// 尝试匹配 API 路由。返回 None 表示不匹配。
async fn try_route_api(
    method: &Method,
    path: &str,
    mut req: Request,
    state: &AppState,
    accept_gzip: bool,
) -> Option<AppResponse> {
    // 健康端点：无鉴权，供固件启动时探测后端 scheme（HTTP vs HTTPS）
    if path == "/api/health" && method == Method::GET {
        return Some(json_response(
            StatusCode::OK,
            &serde_json::json!({"status":"ok","version":"0.3.1"}),
            accept_gzip,
        ));
    }
    if let Some(rest) = path.strip_prefix("/api/device/") {
        // rest 形如 "{deviceId}/{action}" 或 "{deviceId}"
        let (device_id, action) = match rest.find('/') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        match action {
            Some("register") if method == Method::POST => {
                let body = read_body(&mut req).await.ok()?;
                return Some(
                    device_api::handle_register(state, device_id, &body, accept_gzip).await,
                );
            }
            Some("poll") if method == Method::GET => {
                return Some(device_api::handle_poll(state, device_id, &req, accept_gzip).await);
            }
            Some("event") if method == Method::POST => {
                return Some(
                    device_api::handle_event(state, device_id, &mut req, accept_gzip).await,
                );
            }
            Some("frame") if method == Method::POST => {
                return Some(frame::handle_frame(state, device_id, &mut req, accept_gzip).await);
            }
            _ => return Some(bad_request("unknown device action", accept_gzip)),
        }
    }
    if path == "/api/devices" && method == Method::GET {
        return Some(devices::handle_get_devices(state, accept_gzip).await);
    }
    if let Some(rest) = path.strip_prefix("/api/control/") {
        if method == Method::POST {
            let body = read_body(&mut req).await.ok()?;
            return Some(control::handle_post_control(state, rest, &body, accept_gzip).await);
        }
    }
    if let Some(rest) = path.strip_prefix("/api/photo/") {
        if method == Method::POST {
            return Some(photo::handle_post_photo(state, rest, accept_gzip).await);
        }
    }
    if let Some(rest) = path.strip_prefix("/api/stream/") {
        if method == Method::GET {
            return Some(stream::handle_get_stream(state, rest).await);
        }
    }
    if let Some(rest) = path.strip_prefix("/api/pwm_cache/") {
        match *method {
            Method::GET => return Some(pwm_cache::handle_get(state, rest, accept_gzip).await),
            Method::POST => {
                let body = read_body(&mut req).await.ok()?;
                return Some(pwm_cache::handle_post(state, rest, &body, accept_gzip).await);
            }
            _ => {}
        }
    }
    None
}

/// 尝试服务静态资源
fn try_serve_static(path: &str, accept_gzip: bool) -> Option<AppResponse> {
    if path.starts_with("/api/") {
        return None;
    }
    let asset = crate::static_files::lookup(path).or_else(|| {
        // SPA 回退：未知路径 → index.html（让前端路由处理）
        if !path.contains('.') {
            crate::static_files::index_html()
        } else {
            None
        }
    })?;
    let mut builder = Response::build(StatusCode::OK);
    builder.content_type(asset.content_type);
    if path.contains('.') {
        builder.insert_header(("Cache-Control", "public, max-age=3600"));
    }
    if accept_gzip && asset.bytes.len() >= 256 {
        // 对大静态资源应用 gzip
        let gz = gzip_bytes(&asset.bytes);
        builder.insert_header(("Content-Encoding", "gzip"));
        Some(builder.body(Bytes::from(gz)).map_into_boxed_body())
    } else {
        Some(builder.body(asset.bytes).map_into_boxed_body())
    }
}

/// 读取请求体（限制 1MB 防止内存爆炸）
pub async fn read_body(req: &mut Request) -> Result<Bytes, std::io::Error> {
    let mut payload = req.take_payload();
    let mut buf = Vec::with_capacity(1024);
    while let Some(chunk) = payload.next().await {
        match chunk {
            Ok(b) => {
                buf.extend_from_slice(&b);
                if buf.len() > 1024 * 1024 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "请求体超过 1MB 限制",
                    ));
                }
            }
            Err(e) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("payload 读取错误: {e}"),
                ))
            }
        }
    }
    Ok(Bytes::from(buf))
}

/// 判断 Accept-Encoding 头是否包含 gzip
pub fn accepts_gzip(value: Option<&str>) -> bool {
    match value {
        Some(v) => v.to_ascii_lowercase().contains("gzip"),
        None => false,
    }
}

/// gzip 压缩字节
pub fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = GzEncoder::new(Vec::with_capacity(data.len() / 4), Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_else(|_| data.to_vec())
}

/// JSON 响应构造辅助（可选 gzip）
pub fn json_response<T: serde::Serialize>(
    status: StatusCode,
    val: &T,
    accept_gzip: bool,
) -> AppResponse {
    let body = serde_json::to_vec(val).unwrap_or_default();
    let mut builder = Response::build(status);
    builder
        .content_type("application/json")
        .insert_header(("Cache-Control", "no-store"));
    if accept_gzip && body.len() >= 256 {
        let gz = gzip_bytes(&body);
        builder.insert_header(("Content-Encoding", "gzip"));
        builder.body(Bytes::from(gz)).map_into_boxed_body()
    } else {
        builder.body(Bytes::from(body)).map_into_boxed_body()
    }
}

/// 404
pub fn not_found(accept_gzip: bool) -> AppResponse {
    json_response(
        StatusCode::NOT_FOUND,
        &serde_json::json!({"error": "not found"}),
        accept_gzip,
    )
}

/// 400
pub fn bad_request(msg: &str, accept_gzip: bool) -> AppResponse {
    json_response(
        StatusCode::BAD_REQUEST,
        &serde_json::json!({"error": msg}),
        accept_gzip,
    )
}

/// 404 设备不存在
pub fn device_not_found(device_id: &str, accept_gzip: bool) -> AppResponse {
    json_response(
        StatusCode::NOT_FOUND,
        &serde_json::json!({"error": "device not found", "deviceId": device_id}),
        accept_gzip,
    )
}

/// 504 拍照超时
pub fn photo_timeout(accept_gzip: bool) -> AppResponse {
    json_response(
        StatusCode::GATEWAY_TIMEOUT,
        &serde_json::json!({"error": "photo timeout"}),
        accept_gzip,
    )
}
