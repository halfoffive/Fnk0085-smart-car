//! 多设备注册表。
//!
//! 使用 `DashMap` 减少锁竞争（单核 1G 内存下避免全局 RwLock 的瓶颈）。
//! 每个 `DeviceEntry` 持有原子状态（在线/最近心跳/PWM 缓存开关）+ UDP 源地址
//! + 视频缓存句柄 + 拍照回执通道。

use crate::video_cache::VideoCache;
use parking_lot::Mutex;
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

/// 拍照完成回执
#[derive(Debug, Clone)]
pub struct PhotoResult {
    pub path: String,
    pub uptime_ms: u64,
}

/// 后端 → 设备的下行指令载体（UDP 监听任务消费后 AEAD 加密回送设备）
#[derive(Debug)]
pub struct OutboundCommand {
    pub device_id: String,
    /// JSON 字节流（按 protocol.md §2 描述）
    pub json: bytes::Bytes,
}

/// 单设备运行态
pub struct DeviceEntry {
    pub device_id: String,
    /// 是否在线（最近心跳在阈值内）
    pub online: AtomicBool,
    /// 最近一次心跳的服务端时间戳（ms）
    pub last_seen_ms: AtomicU64,
    /// 设备 UDP 源地址（用于下发指令）
    pub addr: Mutex<Option<SocketAddr>>,
    /// 视频缓存
    pub video: VideoCache,
    /// PWM 缓存开关
    pub pwm_cache_enabled: AtomicBool,
    /// 等待中的拍照回执 oneshot（同一设备同时仅允许一个挂起拍照）
    pub photo_pending: Mutex<Option<oneshot::Sender<PhotoResult>>>,
}

impl DeviceEntry {
    pub fn new(device_id: String, video_cache_capacity: usize, video_max_recent: usize) -> Arc<Self> {
        Arc::new(Self {
            device_id,
            online: AtomicBool::new(true),
            last_seen_ms: AtomicU64::new(now_ms()),
            addr: Mutex::new(None),
            video: VideoCache::new(video_cache_capacity, video_max_recent),
            pwm_cache_enabled: AtomicBool::new(true),
            photo_pending: Mutex::new(None),
        })
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn last_seen(&self) -> u64 {
        self.last_seen_ms.load(Ordering::Relaxed)
    }

    pub fn set_addr(&self, addr: SocketAddr) {
        *self.addr.lock() = Some(addr);
    }

    pub fn get_addr(&self) -> Option<SocketAddr> {
        *self.addr.lock()
    }

    pub fn touch(&self, addr: Option<SocketAddr>, ts: u64) {
        self.online.store(true, Ordering::Relaxed);
        self.last_seen_ms.store(ts, Ordering::Relaxed);
        if let Some(a) = addr {
            self.set_addr(a);
        }
    }

    pub fn pwm_cache_enabled(&self) -> bool {
        self.pwm_cache_enabled.load(Ordering::Relaxed)
    }

    pub fn set_pwm_cache(&self, enabled: bool) {
        self.pwm_cache_enabled.store(enabled, Ordering::Relaxed);
    }

    /// 注册一个等待中的拍照回执 oneshot。
    /// 若已有挂起，返回 false（前端应串行化拍照请求）。
    pub fn await_photo(&self, tx: oneshot::Sender<PhotoResult>) -> bool {
        let mut guard = self.photo_pending.lock();
        if guard.is_some() {
            return false;
        }
        *guard = Some(tx);
        true
    }

    /// 收到 photo_done 时调用，完成挂起的 oneshot。
    pub fn complete_photo(&self, result: PhotoResult) -> bool {
        let mut guard = self.photo_pending.lock();
        if let Some(tx) = guard.take() {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }
}

/// 设备列表项（HTTP /api/devices 返回）
#[derive(Debug, Serialize)]
pub struct DeviceSummary {
    pub device_id: String,
    pub online: bool,
    pub last_seen_ms: u64,
}

/// 设备注册表错误
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("超出最大设备数 {max}")]
    MaxDevicesExceeded { max: u32 },
}

/// 设备注册表
pub struct DeviceRegistry {
    inner: dashmap::DashMap<String, Arc<DeviceEntry>>,
    max_devices: u32,
    /// 视频缓存配置（创建新设备时使用）
    video_capacity: usize,
    video_max_recent: usize,
}

impl DeviceRegistry {
    pub fn new(max_devices: u32, video_capacity: usize, video_max_recent: usize) -> Self {
        Self {
            inner: dashmap::DashMap::new(),
            max_devices,
            video_capacity,
            video_max_recent,
        }
    }

    /// 取或创建设备项（收到任何合法上行帧时调用）。
    /// 返回设备句柄；超过最大设备数时返回错误。
    pub fn get_or_create(&self, device_id: &str) -> Result<Arc<DeviceEntry>, RegistryError> {
        if let Some(e) = self.inner.get(device_id) {
            return Ok(e.clone());
        }
        if self.inner.len() >= self.max_devices as usize && !self.inner.contains_key(device_id) {
            return Err(RegistryError::MaxDevicesExceeded {
                max: self.max_devices,
            });
        }
        let entry = DeviceEntry::new(
            device_id.to_string(),
            self.video_capacity,
            self.video_max_recent,
        );
        // entry().or_insert() 返回 &mut Reference，clone 后释放
        let arc = self
            .inner
            .entry(device_id.to_string())
            .or_insert(entry)
            .clone();
        Ok(arc)
    }

    pub fn get(&self, device_id: &str) -> Option<Arc<DeviceEntry>> {
        self.inner.get(device_id).map(|e| e.clone())
    }

    pub fn list(&self) -> Vec<DeviceSummary> {
        self.inner
            .iter()
            .map(|e| DeviceSummary {
                device_id: e.device_id.clone(),
                online: e.is_online(),
                last_seen_ms: e.last_seen(),
            })
            .collect()
    }

    /// 标记超时设备离线（心跳超过阈值）。
    pub fn mark_offline_older_than(&self, threshold_ms: u64, now_ms: u64) {
        for entry in self.inner.iter() {
            if now_ms.saturating_sub(entry.last_seen()) > threshold_ms {
                entry.online.store(false, Ordering::Relaxed);
            }
        }
    }
}

/// 出站指令通道句柄（handlers 与 UDP 监听任务之间的桥）
#[derive(Clone)]
pub struct CommandSink {
    pub tx: mpsc::Sender<OutboundCommand>,
}

impl CommandSink {
    pub fn new(tx: mpsc::Sender<OutboundCommand>) -> Self {
        Self { tx }
    }

    pub async fn send(&self, cmd: OutboundCommand) -> Result<(), mpsc::error::SendError<OutboundCommand>> {
        self.tx.send(cmd).await
    }
}

/// 当前 wall-clock 毫秒数
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_create_basic() {
        let reg = DeviceRegistry::new(8, 8, 4);
        let a = reg.get_or_create("dev1").unwrap();
        let b = reg.get_or_create("dev1").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let c = reg.get_or_create("dev2").unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn enforces_max_devices() {
        let reg = DeviceRegistry::new(1, 8, 4);
        let _ = reg.get_or_create("dev1").unwrap();
        let err = reg.get_or_create("dev2").unwrap_err();
        assert!(matches!(err, RegistryError::MaxDevicesExceeded { max: 1 }));
    }

    #[test]
    fn photo_pending_is_one_shot() {
        let entry = DeviceEntry::new("dev1".into(), 8, 4);
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        assert!(entry.await_photo(tx1));
        assert!(!entry.await_photo(tx2));
    }
}
