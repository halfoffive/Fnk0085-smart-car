// 与后端协议对应的 TypeScript 类型定义（参考 protocol.md §4 HTTP API）

/** 设备项 — GET /api/devices */
export interface Device {
  /** 设备身份，形如 `ESP32S3_<chipId>_<MAC>` */
  deviceId: string;
  /** 在线状态 */
  online: boolean;
  /** 最近一次心跳时间戳（ms） */
  lastSeenMs: number;
}

/** 方向枚举 — 控制指令 */
export type Direction = 'W' | 'A' | 'S' | 'D' | 'stop';

/** POST /api/control/{deviceId} 请求体 */
export interface ControlRequest {
  direction: Direction;
  /** PWM 0..255 */
  pwm: number;
}

/** POST /api/control/{deviceId} 响应体 */
export interface ControlResponse {
  ok: boolean;
}

/** POST /api/photo/{deviceId} 响应体 */
export interface PhotoResponse {
  path: string;
}

/** PWM 缓存档位条目 */
export interface PwmCacheEntry {
  speed: string;
  pwm: number;
}

/** GET /api/pwm_cache/{deviceId} 响应体 */
export interface PwmCacheState {
  enabled: boolean;
  entries: PwmCacheEntry[];
}

/** POST /api/pwm_cache/{deviceId} 请求体 */
export interface PwmCacheToggleRequest {
  enabled: boolean;
}

/** Web Worker → 主线程消息 */
export type WorkerMessage =
  | { type: 'frame'; blob: Blob; latencyMs: number; frameSeq: number }
  | { type: 'latency'; latencyMs: number }
  | { type: 'status'; state: 'connecting' | 'streaming' | 'stopped' | 'error'; message?: string }
  | { type: 'error'; message: string };

/** 主线程 → Worker 消息 */
export type WorkerCommand =
  | { type: 'start'; deviceId: string; apiBase: string }
  | { type: 'stop' }
  | { type: 'pause' }
  | { type: 'resume' };
