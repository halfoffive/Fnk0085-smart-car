//! UDP 监听任务：接收设备分包 + AEAD 解密 + 重组 + 事件分发，
//! 以及出站指令（handlers → 设备）的加密回送。
//!
//! 设计要点：
//! - 入站循环单任务消费 UdpSocket（避免多 worker 抢占同一 socket）。
//! - 出站指令通过 mpsc 通道从 handlers 流入，独立任务做 AEAD 加密后回送。
//! - FrameReassembler 每设备一个，存放在 listener 任务本地 HashMap（避免锁）。

use crate::crypto::{self, AeadKey};
use crate::device::{now_ms, DeviceRegistry, OutboundCommand, PhotoResult};
use crate::protocol::{parse_packet, DeviceEvent, FrameReassembler};
use crate::video_cache::Frame;
use anyhow::{Context, Result};
use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// 启动 UDP 监听 + 出站指令循环。
///
/// 返回时通常表示监听失败（端口占用等）。
pub async fn run(
    udp_addr: String,
    expected_token: Arc<String>,
    key: Arc<AeadKey>,
    registry: Arc<DeviceRegistry>,
    cmd_rx: mpsc::Receiver<OutboundCommand>,
) -> Result<()> {
    let sock = UdpSocket::bind(&udp_addr)
        .await
        .with_context(|| format!("UDP bind {udp_addr} 失败"))?;
    log::info!("UDP 监听已就绪：{udp_addr}");
    let sock = Arc::new(sock);

    // 出站任务
    let out_sock = sock.clone();
    let out_key = key.clone();
    let out_reg = registry.clone();
    tokio::spawn(async move {
        outbound_loop(out_sock, out_key, out_reg, cmd_rx).await;
    });

    // 入站循环（占用当前任务）
    inbound_loop(sock, key, registry, expected_token).await
}

/// 入站：循环 recv_from → AEAD 解密 → 视频分包或 JSON 事件分发。
async fn inbound_loop(
    sock: Arc<UdpSocket>,
    key: Arc<AeadKey>,
    registry: Arc<DeviceRegistry>,
    expected_token: Arc<String>,
) -> Result<()> {
    let mut buf = vec![0u8; 65536];
    // 每设备一个重组器（本地状态，避免锁）
    let mut reassemblers: HashMap<String, FrameReassembler> = HashMap::new();
    let mut idle_ticks: u64;

    loop {
        let (n, src) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("UDP recv_from 错误: {e}");
                continue;
            }
        };
        idle_ticks = 0;
        let wire = &buf[..n];

        // AEAD 解密；失败直接丢弃（避免伪造流量放大）
        let plain = match crypto::open(wire, &key) {
            Ok(p) => p,
            Err(_) => {
                log::debug!("UDP 包 AEAD 解密失败 (from={src})");
                continue;
            }
        };

        // 先尝试作为视频子包解析
        if let Ok(pkt) = parse_packet(&plain) {
            handle_video_packet(&pkt, src, &registry, &mut reassemblers);
            continue;
        }
        // 否则作为 JSON 事件解析
        match serde_json::from_slice::<DeviceEvent>(&plain) {
            Ok(event) => handle_event(event, src, &registry, &expected_token),
            Err(e) => {
                log::debug!("无法解析 UDP 明文（既非视频也非事件）：{e}");
            }
        }

        // 偶尔清理重组器（防止僵尸帧占内存）
        idle_ticks += 1;
        if idle_ticks % 1024 == 0 {
            for r in reassemblers.values_mut() {
                r.evict_older_than(now_ms());
            }
        }
    }
}

/// 处理视频子包：touch 设备 + 重组 + 完整帧入缓存。
fn handle_video_packet(
    pkt: &crate::protocol::Packet,
    src: SocketAddr,
    registry: &DeviceRegistry,
    reassemblers: &mut HashMap<String, FrameReassembler>,
) {
    let entry = match registry.get_or_create(&pkt.device_id) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("设备 {} 注册失败: {e}", pkt.device_id);
            return;
        }
    };
    entry.touch(Some(src), now_ms());

    let reassembler = reassemblers.entry(pkt.device_id.clone()).or_default();
    if let Some(full_jpeg) = reassembler.push(pkt) {
        let frame = Frame {
            uptime_ms: pkt.uptime_ms,
            server_recv_ms: now_ms(),
            jpeg: Bytes::from(full_jpeg),
        };
        entry.video.push(frame);
    }
}

/// 处理 JSON 事件：register / photo_done / ack / error
fn handle_event(
    event: DeviceEvent,
    src: SocketAddr,
    registry: &DeviceRegistry,
    expected_token: &str,
) {
    match event {
        DeviceEvent::Register { device_id, token } => {
            // 校验 token 绑定（支持 "Bearer xxx" 与裸 token 两种格式）
            if !validate_token(&token, expected_token) {
                log::warn!("设备 {device_id} register token 校验失败（来自 {src}）");
                return;
            }
            match registry.get_or_create(&device_id) {
                Ok(entry) => {
                    entry.touch(Some(src), now_ms());
                    log::info!("设备 {device_id} 注册成功");
                }
                Err(e) => log::warn!("设备 {device_id} 注册失败: {e}"),
            }
        }
        DeviceEvent::PhotoDone { path, uptime_ms } => {
            // 注意：JSON 事件未携带 deviceId，需依赖 src → 设备反查
            // 简化：遍历最近一次心跳匹配 src 的设备
            if let Some(entry) = find_device_by_addr(registry, src) {
                let ok = entry.complete_photo(PhotoResult { path: path.clone(), uptime_ms });
                if ok {
                    log::info!("设备 {} 拍照完成: {path}", entry.device_id);
                } else {
                    log::debug!("设备 {} 收到 photo_done 但无挂起请求", entry.device_id);
                }
            } else {
                log::debug!("photo_done 来源未匹配到设备: {src}");
            }
        }
        DeviceEvent::Ack { ref_seq } => {
            log::debug!("收到 ack refSeq={ref_seq} from {src}");
        }
        DeviceEvent::Error { code, message } => {
            log::warn!("设备错误 code={code} message={message} from {src}");
        }
    }
}

/// 校验 register token：与配置 token 比对，允许 "Bearer " 前缀。
fn validate_token(received: &str, expected: &str) -> bool {
    let trimmed = received.strip_prefix("Bearer ").unwrap_or(received);
    trimmed == expected
}

/// 通过 UDP 源地址反查设备（仅用于 photo_done 等 JSON 事件）。
fn find_device_by_addr(registry: &DeviceRegistry, addr: SocketAddr) -> Option<std::sync::Arc<crate::device::DeviceEntry>> {
    for entry in registry.list().into_iter() {
        if let Some(e) = registry.get(&entry.device_id) {
            if e.get_addr() == Some(addr) {
                return Some(e);
            }
        }
    }
    None
}

/// 出站循环：消费 CommandSink 投递的指令 → AEAD 加密 → 发送至设备 UDP 地址。
async fn outbound_loop(
    sock: Arc<UdpSocket>,
    key: Arc<AeadKey>,
    registry: Arc<DeviceRegistry>,
    mut cmd_rx: mpsc::Receiver<OutboundCommand>,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        let entry = match registry.get(&cmd.device_id) {
            Some(e) => e,
            None => {
                log::debug!("出站指令丢弃：未知设备 {}", cmd.device_id);
                continue;
            }
        };
        let addr = match entry.get_addr() {
            Some(a) => a,
            None => {
                log::debug!("出站指令丢弃：设备 {} 无 UDP 地址", cmd.device_id);
                continue;
            }
        };
        // 生成全局唯一 nonce（计数器 + 时间戳，避免密钥重用）
        let nonce = generate_nonce();
        let sealed = key.seal(&nonce, &cmd.json);
        if let Err(e) = sock.send_to(&sealed, addr).await {
            log::warn!("发送 UDP 指令至 {addr} 失败: {e}");
        }
    }
    log::info!("出站指令通道已关闭");
}

/// 全局出站 nonce 计数器（原子，无锁）
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成 12B nonce：低 8B 计数器 + 高 4B 时间戳低 32 位
fn generate_nonce() -> [u8; 12] {
    let c = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&c.to_le_bytes());
    nonce[8..].copy_from_slice(&now_ns.to_le_bytes()[..4]);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation() {
        assert!(validate_token("change-me-please", "change-me-please"));
        assert!(validate_token("Bearer change-me-please", "change-me-please"));
        assert!(!validate_token("wrong", "change-me-please"));
        assert!(!validate_token("Bearer wrong", "change-me-please"));
    }

    #[test]
    fn nonces_are_unique() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }
}
