// fetch 封装 — 所有 HTTP API 调用集中于此（参考 protocol.md §4）

import { API_BASE } from './constants';
import type {
  ControlRequest,
  ControlResponse,
  Device,
  PhotoResponse,
  PwmCacheState,
  PwmCacheToggleRequest,
} from '../types/protocol';

/** 拼接完整 URL（API_BASE 为空时即同源） */
function url(path: string): string {
  return `${API_BASE}${path}`;
}

/** 统一错误处理 */
async function handle<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }
  return (await res.json()) as T;
}

/** GET /api/devices — 设备列表 */
export async function getDevices(signal?: AbortSignal): Promise<Device[]> {
  const res = await fetch(url('/api/devices'), {
    method: 'GET',
    headers: { Accept: 'application/json' },
    signal,
  });
  return handle<Device[]>(res);
}

/** POST /api/control/{deviceId} — 下发 WASD + PWM */
export async function postControl(
  deviceId: string,
  body: ControlRequest,
  signal?: AbortSignal,
): Promise<ControlResponse> {
  const res = await fetch(url(`/api/control/${encodeURIComponent(deviceId)}`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  });
  return handle<ControlResponse>(res);
}

/** POST /api/photo/{deviceId} — 触发拍照并等待回执 */
export async function postPhoto(
  deviceId: string,
  signal?: AbortSignal,
): Promise<PhotoResponse> {
  const res = await fetch(url(`/api/photo/${encodeURIComponent(deviceId)}`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ quality: 'max' }),
    signal,
  });
  return handle<PhotoResponse>(res);
}

/** GET /api/pwm_cache/{deviceId} — 查询 PWM 缓存状态 */
export async function getPwmCache(
  deviceId: string,
  signal?: AbortSignal,
): Promise<PwmCacheState> {
  const res = await fetch(url(`/api/pwm_cache/${encodeURIComponent(deviceId)}`), {
    method: 'GET',
    headers: { Accept: 'application/json' },
    signal,
  });
  return handle<PwmCacheState>(res);
}

/** POST /api/pwm_cache/{deviceId} — 开关 PWM 缓存 */
export async function postPwmCache(
  deviceId: string,
  body: PwmCacheToggleRequest,
  signal?: AbortSignal,
): Promise<ControlResponse> {
  const res = await fetch(url(`/api/pwm_cache/${encodeURIComponent(deviceId)}`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  });
  return handle<ControlResponse>(res);
}

/** 滑块值 → PWM 软件映射：1-100 线性映射至 0-255 */
export function sliderToPwm(value: number): number {
  const clamped = Math.max(1, Math.min(100, value));
  return Math.round((clamped / 100) * 255);
}
