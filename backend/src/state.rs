//! 全局共享状态：handlers 与 UDP 监听任务之间的不可变上下文。
//!
//! 设计：`AppState` 为不可变克隆句柄（内部全部 `Arc`），handlers 与 UDP 监听任务
//! 各持一份副本。所有可变状态封装在 `DeviceRegistry`（DashMap）与 `VideoCache`
//! （broadcast）内部，遵循"函数式风格 + 必要处用并发原语"的约束。

use crate::crypto::AeadKey;
use crate::device::{CommandSink, DeviceRegistry};
use std::sync::Arc;

/// 应用全局状态（不可变，Clone 廉价）
#[derive(Clone)]
pub struct AppState {
    /// 设备注册表（DashMap 内部并发安全）
    pub registry: Arc<DeviceRegistry>,
    /// AEAD 密钥句柄（用于解密/加密设备 UDP 包）
    pub key: Arc<AeadKey>,
    /// 期望的 token（用于 register 校验，可带 "Bearer " 前缀）
    pub expected_token: Arc<String>,
    /// 出站指令通道（handlers → UDP 出站循环）
    pub cmd_sink: CommandSink,
}
