//! 每设备 1s 视频帧环形缓存。
//!
//! 设计：`broadcast` 通道做多订阅者扇出（多个 HTTP 流订阅者共享同一帧源，
//! 避免向设备重复请求），并保留最近若干帧以供诊断或“取最新帧”场景。
//! `Frame` 内 `jpeg` 使用 `bytes::Bytes` 零拷贝克隆。

use bytes::Bytes;
use parking_lot::RwLock;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast;

/// 单帧视频（设备 uptime + JPEG 字节 + 服务端接收时间戳）
#[derive(Debug, Clone)]
pub struct Frame {
    /// 设备系统运行时间（ms），用于估算端到端延迟
    pub uptime_ms: u64,
    /// 服务端接收此帧时的 wall-clock ms
    pub server_recv_ms: u64,
    /// JPEG 完整字节
    pub jpeg: Bytes,
}

/// 视频帧缓存句柄（每设备一个）
///
/// - `tx` 为广播发送端，多订阅者共享。
/// - `recent` 为最近 1s 帧环形缓存（仅保留前若干帧用于“最新帧”查询）。
pub struct VideoCache {
    tx: broadcast::Sender<Arc<Frame>>,
    recent: Arc<RwLock<VecDeque<Arc<Frame>>>>,
    max_recent: usize,
}

impl VideoCache {
    /// 创建缓存，`capacity` 为广播通道容量（决定慢订阅者能容忍多少积压），
    /// `max_recent` 为环形缓存保留的最大帧数。
    pub fn new(capacity: usize, max_recent: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            tx,
            recent: Arc::new(RwLock::new(VecDeque::with_capacity(max_recent))),
            max_recent,
        }
    }

    /// 投递一帧。若所有订阅者已断开则只更新缓存。
    pub fn push(&self, frame: Frame) {
        let arc = Arc::new(frame);
        // 缓存最近帧
        {
            let mut q = self.recent.write();
            q.push_back(arc.clone());
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

/// 视频帧摘要（用于 HTTP /api/diagnostics 等）
#[derive(Debug, Serialize)]
pub struct FrameSummary {
    pub uptime_ms: u64,
    pub server_recv_ms: u64,
    pub jpeg_len: usize,
}

impl From<&Frame> for FrameSummary {
    fn from(f: &Frame) -> Self {
        Self {
            uptime_ms: f.uptime_ms,
            server_recv_ms: f.server_recv_ms,
            jpeg_len: f.jpeg.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_broadcasts_to_subscribers() {
        let cache = VideoCache::new(8, 4);
        let mut sub1 = cache.subscribe();
        let mut sub2 = cache.subscribe();
        cache.push(Frame {
            uptime_ms: 1,
            server_recv_ms: 100,
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
        let cache = VideoCache::new(8, 3);
        for i in 0..10u64 {
            cache.push(Frame {
                uptime_ms: i,
                server_recv_ms: i,
                jpeg: Bytes::from_static(b"x"),
            });
        }
        let latest = cache.latest().unwrap();
        assert_eq!(latest.uptime_ms, 9);
        assert_eq!(cache.recent.read().len(), 3);
    }
}
