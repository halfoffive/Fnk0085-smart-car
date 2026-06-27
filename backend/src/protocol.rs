//! 通信协议：设备 ↔ 后端 ↔ 前端的 JSON 事件 / 指令类型。
//!
//! 视频帧已迁至 HTTPS POST `/api/device/{id}/frame`（二进制 JPEG body），
//! 原 UDP 分包（magic / version / partIdx 等）已废弃，本文件仅保留
//! 控制通道使用的 `DeviceEvent` 与 `DeviceCommand` 两个 serde 枚举。
//!
//! 命名约定（遵循 `protocol.md`）：
//! - variant 名：`snake_case`（serde `rename_all = "snake_case"`）
//! - 字段名：`camelCase`（serde `rename_all_fields = "camelCase"`）
//! - 标签字段：`type`（serde `tag = "type"`，内部标签模式）
//!
//! 例：`Register { device_id, token }` ↔ JSON `{"type":"register","deviceId":...}`。

use serde::{Deserialize, Serialize};

/// 设备 → 后端事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DeviceEvent {
    /// 设备注册（仅应在 /register 端点使用，event 端点拒绝）
    Register { device_id: String, token: String },
    /// 拍照完成回执
    PhotoDone { path: String, uptime_ms: u64 },
    /// 长轮询 Ping 应答
    Ack { ref_seq: u32 },
    /// 设备上报的左右轮速（单位：RPM）
    Telemetry { left_rpm: i32, right_rpm: i32 },
    /// 通用错误上报
    Error { code: i32, message: String },
}

/// 后端 → 设备指令
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
