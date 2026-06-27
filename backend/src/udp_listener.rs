//! UDP 监听任务：仅负责设备视频分包接收 + AEAD 解密 + 重组。
//!
//! 设计要点：
//! - 入站循环单任务消费 UdpSocket（避免多 worker 抢占同一 socket）。
//! - 指令下发已迁至 HTTPS 长轮询（见 handlers/device_api.rs），不再走 UDP 出站。
//! - 设备 JSON 事件（register/photo_done/ack/error）也迁至 HTTPS。
//! - FrameReassembler 每设备一个，存放在 listener 任务本地 HashMap（避免锁）。

use crate::crypto::{self, AeadKey};
use crate::device::{now_ms, DeviceRegistry};
use crate::protocol::{parse_packet, FrameReassembler};
use crate::video_cache::Frame;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// 启动 UDP 监听（仅视频流）。
///
/// 返回时通常表示监听失败（端口占用等）。
pub async fn run(udp_addr: String, key: Arc<AeadKey>, registry: Arc<DeviceRegistry>) -> Result<()> {
    let sock = UdpSocket::bind(&udp_addr)
        .await
        .with_context(|| format!("UDP bind {udp_addr} 失败"))?;
    log::info!("UDP 监听已就绪：{udp_addr}");
    let sock = Arc::new(sock);

    inbound_loop(sock, key, registry).await
}

/// 入站：循环 recv_from → AEAD 解密 → 视频分包重组。
async fn inbound_loop(
    sock: Arc<UdpSocket>,
    key: Arc<AeadKey>,
    registry: Arc<DeviceRegistry>,
) -> Result<()> {
    let mut buf = vec![0u8; 65536];
    // 每设备一个重组器（本地状态，避免锁）
    let mut reassemblers: HashMap<String, FrameReassembler> = HashMap::new();
    // 自上次重组器清理以来收到的包数；每 1024 包清理一次防内存泄漏
    let mut ticks_since_evict: u64 = 0;

    loop {
        let (n, src) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("UDP recv_from 错误: {e}");
                continue;
            }
        };
        let wire = &buf[..n];

        // AEAD 解密；失败直接丢弃（避免伪造流量放大）
        let plain = match crypto::open(wire, &key) {
            Ok(p) => p,
            Err(_) => {
                log::debug!("UDP 包 AEAD 解密失败 (from={src})");
                continue;
            }
        };

        // 视频子包解析；非视频包（理论上不存在，事件已迁 HTTPS）记录后丢弃
        match parse_packet(&plain) {
            Ok(pkt) => handle_video_packet(&pkt, src, &registry, &mut reassemblers),
            Err(e) => log::debug!("UDP 明文非视频包（from={src}）：{e}"),
        }

        // 偶尔清理重组器（防止僵尸帧占内存）
        ticks_since_evict = ticks_since_evict.wrapping_add(1);
        if ticks_since_evict.is_multiple_of(1024) {
            for r in reassemblers.values_mut() {
                r.evict_older_than(now_ms());
            }
        }
    }
}

/// 处理视频子包：touch 设备 + 重组 + 完整帧入缓存。
fn handle_video_packet(
    pkt: &crate::protocol::Packet,
    src: std::net::SocketAddr,
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
    if let Some(jpeg) = reassembler.push(pkt) {
        let frame = Frame {
            server_recv_ms: now_ms(),
            jpeg,
        };
        entry.video.push(frame);
    }
}

#[cfg(test)]
mod tests {
    // udp_listener 现仅做视频包转发，逻辑全部委托给 protocol::parse_packet 与
    // FrameReassembler，单元测试覆盖在 protocol.rs 中。
}
