//! 多设备注册表。
//!
//! 使用 `DashMap` 减少锁竞争（单核 1G 内存下避免全局 RwLock 的瓶颈）。
//! 每个 `DeviceEntry` 持有原子状态（在线/最近心跳/PWM 缓存开关）+ UDP 源地址
//! + 视频缓存句柄 + 拍照回执通道 + 指令队列（HTTPS 长轮询消费）。

use crate::video_cache::VideoCache;
use dashmap::mapref::entry::Entry;
use parking_lot::Mutex;
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

/// 拍照完成回执
/// - ok=true: 拍照成功，path 为 SD 上的 JPEG 路径
/// - ok=false: 设备侧失败（DeviceEvent::Error 触发），path 为空
#[derive(Debug, Clone)]
pub struct PhotoResult {
    pub path: String,
    pub ok: bool,
}

/// 单设备运行态
pub struct DeviceEntry {
    pub device_id: String,
    /// 是否在线（最近心跳在阈值内）
    pub online: AtomicBool,
    /// 最近一次心跳的服务端时间戳（ms）
    pub last_seen_ms: AtomicU64,
    /// 设备 UDP 源地址（仅用于设备列表/调试，指令下发已迁至 HTTPS）
    pub addr: Mutex<Option<SocketAddr>>,
    /// 视频缓存
    pub video: VideoCache,
    /// PWM 缓存开关
    pub pwm_cache_enabled: AtomicBool,
    /// 左轮转速（RPM）
    pub left_rpm: AtomicI32,
    /// 右轮转速（RPM）
    pub right_rpm: AtomicI32,
    /// 等待中的拍照回执 oneshot（同一设备同时仅允许一个挂起拍照）
    pub photo_pending: Mutex<Option<oneshot::Sender<PhotoResult>>>,
    /// 指令队列发送端（handlers 推入指令 JSON）
    pub cmd_tx: mpsc::Sender<bytes::Bytes>,
    /// 指令队列接收端（HTTPS 长轮询端点消费；同一设备同一时刻仅允许一个 poll 持锁）
    pub cmd_rx: AsyncMutex<mpsc::Receiver<bytes::Bytes>>,
}

impl DeviceEntry {
    pub fn new(
        device_id: String,
        video_cache_capacity: usize,
        video_max_recent: usize,
        video_max_bytes: usize,
        video_max_frame_bytes: usize,
    ) -> Arc<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        Arc::new(Self {
            device_id,
            online: AtomicBool::new(true),
            last_seen_ms: AtomicU64::new(now_ms()),
            addr: Mutex::new(None),
            video: VideoCache::new(
                video_cache_capacity,
                video_max_recent,
                video_max_bytes,
                video_max_frame_bytes,
            ),
            pwm_cache_enabled: AtomicBool::new(true),
            left_rpm: AtomicI32::new(0),
            right_rpm: AtomicI32::new(0),
            photo_pending: Mutex::new(None),
            cmd_tx,
            cmd_rx: AsyncMutex::new(cmd_rx),
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

    /// 更新轮速 telemetry。
    pub fn set_telemetry(&self, left_rpm: i32, right_rpm: i32) {
        self.left_rpm.store(left_rpm, Ordering::Relaxed);
        self.right_rpm.store(right_rpm, Ordering::Relaxed);
    }

    /// 读取当前轮速遥测值。
    pub fn telemetry(&self) -> (i32, i32) {
        (
            self.left_rpm.load(Ordering::Relaxed),
            self.right_rpm.load(Ordering::Relaxed),
        )
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

    /// 推入下行指令（非阻塞，队列满时返回错误由调用方记录丢弃）。
    pub fn try_push_command(
        &self,
        json: bytes::Bytes,
    ) -> Result<(), mpsc::error::TrySendError<bytes::Bytes>> {
        self.cmd_tx.try_send(json)
    }
}

/// 设备列表项（HTTP /api/devices 返回）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSummary {
    pub device_id: String,
    pub online: bool,
    pub last_seen_ms: u64,
    pub left_rpm: i32,
    pub right_rpm: i32,
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
    /// 当前已创建设备数（上限控制，不减少）
    device_count: AtomicUsize,
    /// 视频缓存配置（创建新设备时使用）
    video_capacity: usize,
    video_max_recent: usize,
    video_max_bytes: usize,
    video_max_frame_bytes: usize,
}

impl DeviceRegistry {
    pub fn new(
        max_devices: u32,
        video_capacity: usize,
        video_max_recent: usize,
        video_max_bytes: usize,
        video_max_frame_bytes: usize,
    ) -> Self {
        Self {
            inner: dashmap::DashMap::new(),
            max_devices,
            device_count: AtomicUsize::new(0),
            video_capacity,
            video_max_recent,
            video_max_bytes,
            video_max_frame_bytes,
        }
    }

    /// 取或创建设备项（收到任何合法上行帧时调用）。
    /// 返回设备句柄；超过最大设备数时返回错误。
    pub fn get_or_create(&self, device_id: &str) -> Result<Arc<DeviceEntry>, RegistryError> {
        match self.inner.entry(device_id.to_string()) {
            Entry::Occupied(e) => Ok(e.get().clone()),
            Entry::Vacant(e) => {
                // CAS 循环申请一个设备名额；成功后再通过 entry 插入，
                // 将“存在性检查 + 名额检查 + 插入”合并为一次 entry 原子操作。
                loop {
                    let current = self.device_count.load(Ordering::Relaxed);
                    if current >= self.max_devices as usize {
                        return Err(RegistryError::MaxDevicesExceeded {
                            max: self.max_devices,
                        });
                    }
                    match self.device_count.compare_exchange_weak(
                        current,
                        current + 1,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                }
                let arc = DeviceEntry::new(
                    device_id.to_string(),
                    self.video_capacity,
                    self.video_max_recent,
                    self.video_max_bytes,
                    self.video_max_frame_bytes,
                );
                e.insert(arc.clone());
                Ok(arc)
            }
        }
    }

    pub fn get(&self, device_id: &str) -> Option<Arc<DeviceEntry>> {
        self.inner.get(device_id).map(|e| e.clone())
    }

    pub fn list(&self) -> Vec<DeviceSummary> {
        self.inner
            .iter()
            .map(|e| {
                let (left_rpm, right_rpm) = e.telemetry();
                DeviceSummary {
                    device_id: e.device_id.clone(),
                    online: e.is_online(),
                    last_seen_ms: e.last_seen(),
                    left_rpm,
                    right_rpm,
                }
            })
            .collect()
    }
}

/// 服务启动时刻（进程生命周期内只初始化一次）
static START_TIME: OnceLock<Instant> = OnceLock::new();

/// 当前单调毫秒数（以进程启动为基准，避免 NTP 回拨影响延迟计算）。
pub fn now_ms() -> u64 {
    let start = START_TIME.get_or_init(Instant::now);
    Instant::now().duration_since(*start).as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_create_basic() {
        let reg = DeviceRegistry::new(8, 8, 4, 1024, 256);
        let a = reg.get_or_create("dev1").unwrap();
        let b = reg.get_or_create("dev1").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let c = reg.get_or_create("dev2").unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn enforces_max_devices() {
        let reg = DeviceRegistry::new(1, 8, 4, 1024, 256);
        let _ = reg.get_or_create("dev1").unwrap();
        let res = reg.get_or_create("dev2");
        assert!(matches!(
            res,
            Err(RegistryError::MaxDevicesExceeded { max: 1 })
        ));
    }

    #[test]
    fn photo_pending_is_one_shot() {
        let entry = DeviceEntry::new("dev1".into(), 8, 4, 1024, 256);
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        assert!(entry.await_photo(tx1));
        assert!(!entry.await_photo(tx2));
    }

    #[test]
    fn concurrent_get_or_create_respects_max_devices() {
        let reg = Arc::new(DeviceRegistry::new(8, 8, 4, 1024, 256));
        let mut handles = Vec::new();
        for i in 0..100 {
            let reg = reg.clone();
            handles.push(std::thread::spawn(move || {
                reg.get_or_create(&format!("dev{i}"))
            }));
        }
        for h in handles {
            let _ = h.join().unwrap();
        }
        assert!(reg.list().len() <= 8, "并发注册不应超过 max_devices=8 上限");
    }
}
