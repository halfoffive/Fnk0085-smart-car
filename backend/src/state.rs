//! 全局共享状态：handlers 之间的不可变上下文。
//!
//! 设计：`AppState` 为不可变克隆句柄（内部全部 `Arc`），各 handler 持有副本。
//! 所有可变状态封装在 `DeviceRegistry`（DashMap）与 `VideoCache`（broadcast）内部，
//! 遵循"函数式风格 + 必要处用并发原语"的约束。

use crate::device::DeviceRegistry;
use std::sync::Arc;

/// 应用全局状态（不可变，Clone 廉价）
#[derive(Clone)]
pub struct AppState {
    /// 设备注册表（DashMap 内部并发安全）
    pub registry: Arc<DeviceRegistry>,
    /// 期望的 token（用于 HTTP register / poll / event / frame 校验）
    pub expected_token: Arc<String>,
    /// 前端访问密码（用于 POST /api/auth/login 校验）
    pub frontend_password: Arc<String>,
    /// 日志级别（生产环境/调试模式控制）
    pub log_level: Arc<String>,
    /// 本服务监听地址（供 /api/config 返回给前端配网）
    pub server_addr: String,
}
