//! 每设备 1s 视频帧环形缓存。
//!
//! 设计：`broadcast` 通道做多订阅者扇出（多个 HTTP 流订阅者共享同一帧源，
//! 避免向设备重复请求），并保留最近若干帧以供诊断或“取最新帧”场景。
//! `Frame` 内 `jpeg` 使用 `bytes::Bytes` 零拷贝克隆。

use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast;

/// 单帧视频（JPEG 字节 + 时间戳）
#[derive(Debug, Clone)]
pub struct Frame {
    /// 服务端接收此帧时的单调毫秒数（进程启动为基准）
    pub server_recv_ms: u64,
    /// 设备上报的运行时间毫秒数（可选，来自 X-Device-Uptime-Ms 头）
    pub device_uptime_ms: Option<u64>,
    /// JPEG 完整字节
    pub jpeg: Bytes,
}

/// 视频帧缓存句柄（每设备一个）
///
/// - `tx` 为广播发送端，多订阅者共享。
/// - `recent` 为最近帧环形缓存（按帧数与字节数双重上限管理）。
pub struct VideoCache {
    tx: broadcast::Sender<Arc<Frame>>,
    recent: Arc<RwLock<VecDeque<Arc<Frame>>>>,
    max_recent: usize,
    /// 每设备缓存总字节上限
    max_bytes: usize,
    /// 单帧大小上限
    max_frame_bytes: usize,
}

impl VideoCache {
    /// 创建缓存。
    /// - `capacity`：广播通道容量（决定慢订阅者能容忍多少积压）。
    /// - `max_recent`：环形缓存保留的最大帧数。
    /// - `max_bytes`：缓存总字节上限。
    /// - `max_frame_bytes`：单帧大小上限，超过直接丢弃。
    pub fn new(
        capacity: usize,
        max_recent: usize,
        max_bytes: usize,
        max_frame_bytes: usize,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            tx,
            recent: Arc::new(RwLock::new(VecDeque::with_capacity(max_recent))),
            max_recent,
            max_bytes,
            max_frame_bytes,
        }
    }

    /// 投递一帧。
    /// - 单帧超过 `max_frame_bytes` 直接丢弃。
    /// - 否则入队，并按 LRU（队首最旧）弹出旧帧直到总字节数 ≤ `max_bytes`。
    /// - 若所有订阅者已断开则只更新缓存。
    pub fn push(&self, frame: Frame) {
        if frame.jpeg.len() > self.max_frame_bytes {
            return;
        }
        let arc = Arc::new(frame);
        // 缓存最近帧，按字节数 LRU 淘汰
        {
            let mut q = self.recent.write();
            q.push_back(arc.clone());
            let mut total: usize = q.iter().map(|f| f.jpeg.len()).sum();
            while total > self.max_bytes && q.len() > 1 {
                if let Some(old) = q.pop_front() {
                    total -= old.jpeg.len();
                }
            }
            // 帧数上限兜底
            while q.len() > self.max_recent {
                q.pop_front();
            }
        }
        // 忽略无订阅者错误
        let _ = self.tx.send(arc);
    }

    /// 订阅新帧流。订阅后从“下一帧”开始接收。
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Frame>> {
        self.tx.subscribe()
    }

    /// 获取最新一帧（若已有帧）。多前端订阅共享同一缓存。
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.recent.read().back().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_broadcasts_to_subscribers() {
        let cache = VideoCache::new(8, 4, 1024, 256);
        let mut sub1 = cache.subscribe();
        let mut sub2 = cache.subscribe();
        cache.push(Frame {
            server_recv_ms: 100,
            device_uptime_ms: None,
            jpeg: Bytes::from_static(b"abc"),
        });
        let f1 = sub1.recv().await.unwrap();
        let f2 = sub2.recv().await.unwrap();
        assert_eq!(f1.jpeg.as_ref(), b"abc");
        assert_eq!(f2.jpeg.as_ref(), b"abc");
        assert_eq!(cache.latest().unwrap().jpeg.as_ref(), b"abc");
    }

    #[test]
    fn recent_caps_at_max() {
        let cache = VideoCache::new(8, 3, 1024, 256);
        for i in 0..10u64 {
            cache.push(Frame {
                server_recv_ms: i,
                device_uptime_ms: None,
                jpeg: Bytes::from_static(b"x"),
            });
        }
        let latest = cache.latest().unwrap();
        assert_eq!(latest.server_recv_ms, 9);
        assert_eq!(cache.recent.read().len(), 3);
    }

    #[test]
    fn drops_oversized_frame() {
        let cache = VideoCache::new(8, 4, 1024, 10);
        cache.push(Frame {
            server_recv_ms: 1,
            device_uptime_ms: None,
            jpeg: Bytes::from(vec![0u8; 11]),
        });
        assert!(
            cache.latest().is_none(),
            "超过 max_frame_bytes 的单帧应被丢弃"
        );
    }

    #[test]
    fn evicts_old_frames_when_total_bytes_exceeds_limit() {
        let cache = VideoCache::new(8, 100, 100, 30);
        for i in 0..5u64 {
            cache.push(Frame {
                server_recv_ms: i,
                device_uptime_ms: None,
                jpeg: Bytes::from(vec![0u8; 30]),
            });
        }
        let q = cache.recent.read();
        let total: usize = q.iter().map(|f| f.jpeg.len()).sum();
        assert!(
            total <= 100,
            "缓存总字节数应不超过 max_bytes=100，实际 {total}"
        );
        // 最新帧必须保留
        assert_eq!(cache.latest().unwrap().server_recv_ms, 4);
    }
}
