//! 通信协议：UDP 视频分包解析 + JSON 事件/指令类型。
//!
//! 严格遵循 protocol.md 字节布局：
//!   magic(2B) | version(1B) | deviceIdLen(1B) | deviceId(NB)
//!   | uptimeMs(8B LE) | frameSeq(4B LE) | partIdx(1B) | partTotal(1B)
//!   | row(1B) | col(1B) | payloadLen(4B LE) | payload(MB)
//!
//! 全部小端序。`parse_packet` 为纯函数；`FrameReassembler` 维护状态用于按
//! (deviceId, frameSeq) 重组 8 分包为完整 JPEG。

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// 协议 magic 字节
pub const MAGIC: [u8; 2] = [0xF1, 0xD0];
/// 协议版本
pub const VERSION: u8 = 1;
/// 单帧分包总数（2 行 × 4 列 = 8）
pub const PART_TOTAL: u8 = 8;

/// 协议解析错误
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("包长度不足，需要至少 {0} 字节")]
    TooShort(usize),
    #[error("magic 错误，期望 {:?} 实际 {:?}", MAGIC, .0)]
    BadMagic([u8; 2]),
    #[error("协议版本不支持，期望 {expected} 实际 {actual}")]
    BadVersion { expected: u8, actual: u8 },
    #[error("partTotal 不合法，期望 {expected} 实际 {actual}")]
    BadPartTotal { expected: u8, actual: u8 },
    #[error("partIdx 越界 {0}")]
    BadPartIdx(u8),
    #[error("payloadLen 越界，声称 {claimed} 实际剩余 {remaining}")]
    PayloadLenMismatch { claimed: usize, remaining: usize },
    #[error("deviceId 长度非法 {0}")]
    BadDeviceIdLen(u8),
}

/// 单个 UDP 子包（已解密后的明文协议帧）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub version: u8,
    pub device_id: String,
    pub uptime_ms: u64,
    pub frame_seq: u32,
    pub part_idx: u8,
    pub part_total: u8,
    pub row: u8,
    pub col: u8,
    pub payload: Bytes,
}

/// 协议帧最小头长度（固定部分，不含 deviceId 与 payload）
/// = magic(2) + version(1) + deviceIdLen(1) + uptime(8) + frameSeq(4)
///   + partIdx(1) + partTotal(1) + row(1) + col(1) + payloadLen(4)
const FIXED_HEADER_LEN: usize = 24;

/// 纯函数：解析单个 UDP 子包字节流。
///
/// 不做解密，输入需为 AEAD 解密后的明文协议字节流。
pub fn parse_packet(input: &[u8]) -> Result<Packet, ProtocolError> {
    if input.len() < FIXED_HEADER_LEN {
        return Err(ProtocolError::TooShort(FIXED_HEADER_LEN));
    }
    if input[0..2] != MAGIC {
        return Err(ProtocolError::BadMagic([input[0], input[1]]));
    }
    let version = input[2];
    if version != VERSION {
        return Err(ProtocolError::BadVersion {
            expected: VERSION,
            actual: version,
        });
    }
    let device_id_len = input[3] as usize;
    if device_id_len == 0 {
        return Err(ProtocolError::BadDeviceIdLen(0));
    }
    let device_id_end = 4 + device_id_len;
    if input.len() < device_id_end + 20 {
        // uptime(8) + frameSeq(4) + partIdx(1) + partTotal(1) + row(1) + col(1) + payloadLen(4) = 20
        return Err(ProtocolError::TooShort(device_id_end + 20));
    }
    let device_id = match std::str::from_utf8(&input[4..device_id_end]) {
        Ok(s) => s.to_string(),
        Err(_) => return Err(ProtocolError::BadDeviceIdLen(device_id_len as u8)),
    };
    let uptime_ms = u64::from_le_bytes(input[device_id_end..device_id_end + 8].try_into().unwrap());
    let frame_seq = u32::from_le_bytes(
        input[device_id_end + 8..device_id_end + 12]
            .try_into()
            .unwrap(),
    );
    let part_idx = input[device_id_end + 12];
    let part_total = input[device_id_end + 13];
    if part_total != PART_TOTAL {
        return Err(ProtocolError::BadPartTotal {
            expected: PART_TOTAL,
            actual: part_total,
        });
    }
    if part_idx >= PART_TOTAL {
        return Err(ProtocolError::BadPartIdx(part_idx));
    }
    let row = input[device_id_end + 14];
    let col = input[device_id_end + 15];
    let payload_len = u32::from_le_bytes(
        input[device_id_end + 16..device_id_end + 20]
            .try_into()
            .unwrap(),
    ) as usize;
    let payload_start = device_id_end + 20;
    let remaining = input.len().saturating_sub(payload_start);
    if remaining < payload_len {
        return Err(ProtocolError::PayloadLenMismatch {
            claimed: payload_len,
            remaining,
        });
    }
    let payload = Bytes::copy_from_slice(&input[payload_start..payload_start + payload_len]);
    Ok(Packet {
        version,
        device_id,
        uptime_ms,
        frame_seq,
        part_idx,
        part_total,
        row,
        col,
        payload,
    })
}

/// 帧重组器：按 (deviceId, frameSeq) 收集 8 个子包，按 partIdx 顺序拼回完整 JPEG。
///
/// 单设备并发使用；不同设备的重组器各自独立。
#[derive(Debug, Default)]
pub struct FrameReassembler {
    /// key = frameSeq，value = (已收到的子包索引 → payload, last_update_ms)
    frames: HashMap<u32, ReassemblingFrame>,
}

#[derive(Debug, Default)]
struct ReassemblingFrame {
    parts: Vec<Option<Bytes>>,
    received: u8,
}

impl FrameReassembler {
    /// 投入一个子包；当 8 个分包全部到齐时返回完整 JPEG（按 partIdx 顺序拼接）。
    pub fn push(&mut self, pkt: &Packet) -> Option<Bytes> {
        let frame = self
            .frames
            .entry(pkt.frame_seq)
            .or_insert_with(|| ReassemblingFrame {
                parts: (0..pkt.part_total).map(|_| None).collect(),
                received: 0,
            });
        let idx = pkt.part_idx as usize;
        if idx >= frame.parts.len() {
            return None;
        }
        if frame.parts[idx].is_none() {
            frame.parts[idx] = Some(pkt.payload.clone());
            frame.received += 1;
        }
        if frame.received == pkt.part_total {
            // 全部到齐，按 partIdx 顺序拼接
            let mut full = Vec::with_capacity(frame.parts.len() * 1024);
            for b in frame.parts.drain(..).flatten() {
                full.extend_from_slice(&b);
            }
            self.frames.remove(&pkt.frame_seq);
            return Some(Bytes::from(full));
        }
        None
    }

    /// 清理超时未完成的帧（防止内存泄漏）
    pub fn evict_older_than(&mut self, _now_ms: u64) {
        // 简化实现：保留最近少量帧，超出阈值时清空
        if self.frames.len() > 32 {
            self.frames.clear();
        }
    }
}

// ===== JSON 事件（设备 → 后端） =====
//
// 使用 serde 的 `rename_all = "snake_case"`（variant 名）+
// `rename_all_fields = "camelCase"`（字段名），匹配 protocol.md 的字段命名。
// 例：`Register { device_id }` → JSON `{"type":"register","deviceId":...}`。

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DeviceEvent {
    Register { device_id: String, token: String },
    PhotoDone { path: String, uptime_ms: u64 },
    Ack { ref_seq: u32 },
    Error { code: i32, message: String },
}

// ===== JSON 指令（后端 → 设备） =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DeviceCommand {
    Control {
        direction: String,
        pwm: u16,
        duration_ms: Option<u32>,
        ts: u64,
    },
    Photo {
        quality: String,
        ts: u64,
    },
    PwmCache {
        enabled: bool,
        ts: u64,
    },
    Ping {
        seq: u32,
        ts: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_packet(part_idx: u8, payload: &[u8]) -> Vec<u8> {
        let device_id = b"ESP32S3_123456_AABBCCDDEEFF";
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.push(device_id.len() as u8);
        out.extend_from_slice(device_id);
        out.extend_from_slice(&12345u64.to_le_bytes());
        out.extend_from_slice(&42u32.to_le_bytes());
        out.push(part_idx);
        out.push(PART_TOTAL);
        out.push(part_idx / 4); // row
        out.push(part_idx % 4); // col
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn parses_single_packet() {
        let bytes = build_packet(0, b"hello");
        let pkt = parse_packet(&bytes).expect("解析成功");
        assert_eq!(pkt.device_id, "ESP32S3_123456_AABBCCDDEEFF");
        assert_eq!(pkt.uptime_ms, 12345);
        assert_eq!(pkt.frame_seq, 42);
        assert_eq!(pkt.part_idx, 0);
        assert_eq!(pkt.part_total, 8);
        assert_eq!(pkt.payload.as_ref(), b"hello");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = build_packet(0, b"x");
        bytes[0] = 0x00;
        assert!(matches!(
            parse_packet(&bytes),
            Err(ProtocolError::BadMagic(_))
        ));
    }

    #[test]
    fn reassembles_full_frame() {
        let mut r = FrameReassembler::default();
        let mut last = None;
        for i in 0..PART_TOTAL {
            let payload = vec![i; 16];
            let bytes = build_packet(i, &payload);
            let pkt = parse_packet(&bytes).unwrap();
            last = r.push(&pkt);
        }
        let full = last.expect("8 分包后产出完整 JPEG");
        assert_eq!(full.len(), (PART_TOTAL as usize) * 16);
    }

    #[test]
    fn handles_out_of_order_parts() {
        let mut r = FrameReassembler::default();
        // 乱序提交
        let order = [3u8, 0, 7, 1, 5, 2, 6, 4];
        let mut produced = None;
        for &i in &order {
            let payload = vec![i; 8];
            let bytes = build_packet(i, &payload);
            let pkt = parse_packet(&bytes).unwrap();
            if let Some(full) = r.push(&pkt) {
                produced = Some(full);
            }
        }
        let full = produced.expect("乱序后仍应产出");
        assert_eq!(full.len(), 8 * 8);
    }
}
