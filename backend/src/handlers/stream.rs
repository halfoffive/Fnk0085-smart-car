//! GET /api/stream/{deviceId} — 视频流推送（multipart/x-mixed-replace 兼容裸 JPEG）。
//!
//! 设计：
//! - 订阅 DeviceEntry.video broadcast。
//! - 每帧 JPEG 作为独立 part 推送，X-Latency-Ms 头携带端到端延迟。
//! - 不压缩（视频流已 JPEG，gzip 收益小于开销）。
//! - 客户端断开时自动结束（broadcast 通道 + Stream 自然完成）。
//!
//! 兼容性：前端 videoWorker 支持 multipart/x-mixed-replace 与裸 JPEG 字节流两种格式。
//! 这里选择 multipart 形式以携带 per-frame latency 头。

use crate::handlers::AppResponse;
use crate::state::AppState;
use actix_http::body::BodyStream;
use actix_http::{Response, StatusCode};
use bytes::Bytes;
use futures::stream;
use futures::Stream;

/// multipart boundary 字符串
const BOUNDARY: &str = "fnk0085frame";

/// 处理 GET /api/stream/{deviceId}
pub async fn handle_get_stream(state: &AppState, device_id: &str) -> AppResponse {
    let entry = match state.registry.get(device_id) {
        Some(e) => e,
        None => {
            // 设备不存在：返回 404 JSON
            return crate::handlers::device_not_found(device_id, false);
        }
    };

    // 计算初始 latency（基于最新帧的 server_recv_ms）
    let initial_latency = entry
        .video
        .latest()
        .map(|f| crate::device::now_ms().saturating_sub(f.server_recv_ms))
        .unwrap_or(0);

    // 订阅 broadcast 流
    let rx = entry.video.subscribe();
    let stream = frame_stream(rx);

    // 构造 multipart 响应
    let mut builder = Response::build(StatusCode::OK);
    builder
        .content_type(format!(
            "multipart/x-mixed-replace; boundary=\"{BOUNDARY}\""
        ))
        .insert_header(("Cache-Control", "no-store"))
        .insert_header(("Connection", "close"))
        .insert_header(("X-Latency-Ms", initial_latency.to_string()));

    builder.body(BodyStream::new(stream)).map_into_boxed_body()
}

/// 将 broadcast Receiver 转换为 multipart part 字节流。
///
/// 每个 part 结构：
/// ```text
/// --<boundary>\r\n
/// Content-Type: image/jpeg\r\n
/// X-Latency-Ms: <n>\r\n
/// \r\n
/// <jpeg bytes>\r\n
/// ```
fn frame_stream(
    rx: tokio::sync::broadcast::Receiver<std::sync::Arc<crate::video_cache::Frame>>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(frame) => {
                    let latency = crate::device::now_ms().saturating_sub(frame.server_recv_ms);
                    let part = build_part(&frame.jpeg, latency);
                    return Some((Ok(part), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // 慢订阅者跳过积压帧
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // 通道关闭：流自然结束
                    return None;
                }
            }
        }
    })
}

/// 构造单个 multipart part 字节
fn build_part(jpeg: &Bytes, latency_ms: u64) -> Bytes {
    let header =
        format!("--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nX-Latency-Ms: {latency_ms}\r\n\r\n");
    let trailer = "\r\n";
    let mut out = Vec::with_capacity(header.len() + jpeg.len() + trailer.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(jpeg);
    out.extend_from_slice(trailer.as_bytes());
    Bytes::from(out)
}
