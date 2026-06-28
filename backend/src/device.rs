//! 多设备注册表。
//!
//! 使用 `DashMap` 减少锁竞争（单核 1G 内存下避免全局 RwLock 的瓶颈）。
//! 每个 `DeviceEntry` 持有原子状态（在线/最近心跳/PWM 缓存开关）+ UDP 源地址
//! + 视频缓存句柄 + 拍照回执通道 + 指令队列（HTTPS 长轮询消费）。

use crate::video_cache::VideoCache;
use dashmap::mapref::entry::Entry;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;

/// 拍照完成回执
/// - ok=true: 拍照成功，path 为 SD 上的 JPEG 路径
/// - ok=false: 设备侧失败（DeviceEvent::Error 触发），path 为空
#[derive(Debug, Clone)]
pub struct PhotoResult {
    pub path: String,
    pub ok: bool,
}

/// 指令队列容量（覆盖旧指令策略）。
const COMMAND_QUEUE_CAPACITY: usize = 16;

/// 设备离线超时（毫秒）：超过此时间未收到任何心跳/帧/事件即标记为离线。
pub const OFFLINE_TIMEOUT_MS: u64 = 30_000;

/// 可覆盖的指令队列：满时丢弃最旧指令，保证新指令总能入队。
pub struct CommandQueue {
    cap: usize,
    buf: std::sync::Mutex<VecDeque<bytes::Bytes>>,
    notify: tokio::sync::Notify,
}

impl CommandQueue {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            buf: std::sync::Mutex::new(VecDeque::with_capacity(cap)),
            notify: tokio::sync::Notify::new(),
        }
    }

    /// 推入指令。若队列已满，弹出最旧指令后再压入。
    /// 返回 `true` 表示发生了丢弃。
    pub fn push(&self, json: bytes::Bytes) -> bool {
        let mut buf = self.buf.lock().unwrap();
        let dropped = if buf.len() >= self.cap {
            buf.pop_front();
            true
        } else {
            false
        };
        buf.push_back(json);
        drop(buf);
        self.notify.notify_one();
        dropped
    }

    /// 等待指令，超时返回 None。
    /// 使用 deadline 控制总等待时间，避免通知丢失窗口导致额外等待。
    pub async fn recv_timeout(&self, dur: Duration) -> Option<bytes::Bytes> {
        let deadline = Instant::now() + dur;
        loop {
            {
                let mut buf = self.buf.lock().unwrap();
                if let Some(json) = buf.pop_front() {
                    return Some(json);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match tokio::time::timeout(remaining, self.notify.notified()).await {
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }
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
    /// 可覆盖指令队列（handlers 推入，HTTPS 长轮询消费）
    pub cmd_queue: Arc<CommandQueue>,
    /// poll 端点互斥锁（同一设备同一时刻仅允许一个 poll 持锁，防止重复消费）
    pub cmd_poll_lock: AsyncMutex<()>,
}

impl DeviceEntry {
    pub fn new(
        device_id: String,
        video_cache_capacity: usize,
        video_max_recent: usize,
        video_max_bytes: usize,
        video_max_frame_bytes: usize,
    ) -> Arc<Self> {
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
            cmd_queue: Arc::new(CommandQueue::new(COMMAND_QUEUE_CAPACITY)),
            cmd_poll_lock: AsyncMutex::new(()),
        })
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn last_seen(&self) -> u64 {
        self.last_seen_ms.load(Ordering::Relaxed)
    }

    /// 若设备超过 OFFLINE_TIMEOUT_MS 未刷新，则标记为离线并返回 true。
    pub fn mark_offline_if_stale(&self, now_ms: u64) -> bool {
        if now_ms.saturating_sub(self.last_seen()) > OFFLINE_TIMEOUT_MS {
            self.online.store(false, Ordering::Relaxed);
            true
        } else {
            false
        }
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

    /// 推入下行指令（队列满时自动覆盖最旧指令，返回是否发生丢弃）。
    pub fn push_command(&self, json: bytes::Bytes) -> bool {
        self.cmd_queue.push(json)
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

    /// 扫描所有设备，将超时的设备标记为离线。
    pub fn sweep_offline(&self) {
        let now = now_ms();
        for entry in self.inner.iter() {
            if entry.mark_offline_if_stale(now) {
                log::debug!("设备 {} 已离线", entry.device_id);
            }
        }
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
    use bytes::Bytes;

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

    #[test]
    fn command_queue_respects_capacity() {
        let q = CommandQueue::new(2);
        assert!(!q.push(Bytes::from_static(b"a")));
        assert!(!q.push(Bytes::from_static(b"b")));
        assert!(q.push(Bytes::from_static(b"c")));
    }

    #[test]
    fn command_queue_drops_oldest_when_full() {
        let q = CommandQueue::new(2);
        q.push(Bytes::from_static(b"old"));
        q.push(Bytes::from_static(b"mid"));
        q.push(Bytes::from_static(b"new"));

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let first = q.recv_timeout(Duration::from_millis(10)).await.unwrap();
            assert_eq!(first, Bytes::from_static(b"mid"));
            let second = q.recv_timeout(Duration::from_millis(10)).await.unwrap();
            assert_eq!(second, Bytes::from_static(b"new"));
        });
    }

    #[test]
    fn command_queue_fifo_order() {
        let q = CommandQueue::new(4);
        q.push(Bytes::from_static(b"1"));
        q.push(Bytes::from_static(b"2"));
        q.push(Bytes::from_static(b"3"));

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            assert_eq!(
                q.recv_timeout(Duration::from_millis(10)).await.unwrap(),
                Bytes::from_static(b"1")
            );
            assert_eq!(
                q.recv_timeout(Duration::from_millis(10)).await.unwrap(),
                Bytes::from_static(b"2")
            );
            assert_eq!(
                q.recv_timeout(Duration::from_millis(10)).await.unwrap(),
                Bytes::from_static(b"3")
            );
        });
    }

    #[test]
    fn command_queue_recv_timeout_returns_none() {
        let q = CommandQueue::new(2);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let res = q.recv_timeout(Duration::from_millis(50)).await;
            assert!(res.is_none());
        });
    }

    #[test]
    fn command_queue_recv_timeout_wakes_on_push() {
        let q = Arc::new(CommandQueue::new(2));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let q2 = q.clone();
            let recv_fut =
                tokio::spawn(async move { q2.recv_timeout(Duration::from_secs(10)).await });
            // 确保 recv 已经注册 wait 后再 push，测试通知唤醒即时性
            tokio::time::sleep(Duration::from_millis(50)).await;
            q.push(Bytes::from_static(b"wake"));
            let start = std::time::Instant::now();
            let res = recv_fut.await.unwrap();
            assert_eq!(res.unwrap(), Bytes::from_static(b"wake"));
            assert!(
                start.elapsed() < Duration::from_millis(500),
                "通知丢失窗口导致额外等待"
            );
        });
    }

    #[test]
    fn mark_offline_if_stale_works() {
        let entry = DeviceEntry::new("dev1".into(), 8, 4, 1024, 256);
        entry.last_seen_ms.store(0, Ordering::Relaxed);
        assert!(entry.mark_offline_if_stale(OFFLINE_TIMEOUT_MS + 1));
        assert!(!entry.is_online());

        let entry2 = DeviceEntry::new("dev2".into(), 8, 4, 1024, 256);
        entry2.last_seen_ms.store(0, Ordering::Relaxed);
        assert!(!entry2.mark_offline_if_stale(OFFLINE_TIMEOUT_MS - 1));
        assert!(entry2.is_online());
    }
}
