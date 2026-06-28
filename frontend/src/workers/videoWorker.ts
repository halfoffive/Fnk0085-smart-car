// 视频流 Worker：多线程读取 chunked / multipart 流并解析 JPEG 帧
// 兼容两种传输：multipart/x-mixed-replace 与裸 JPEG 字节流
// 解析每帧后测算延时（X-Latency-Ms 头 + 帧到达间隔），通过 transferable 传回主线程

/// <reference lib="webworker" />

import type { WorkerCommand, WorkerMessage } from '../types/protocol';
import { isValidDeviceId } from '../lib/validate';

let abortController: AbortController | null = null;
let paused = false;
let frameSeq = 0;

// 缓冲区上界：避免后端误推大 chunk 时内存爆炸
const MAX_BUFFER = 1024 * 1024; // 1 MiB

self.addEventListener('message', (e: MessageEvent<WorkerCommand>) => {
  const msg = e.data;
  switch (msg.type) {
    case 'start': {
      // deviceId 合法性校验 — 拦截 option textContent 误传导致 404
      if (!isValidDeviceId(msg.deviceId)) {
        post({
          type: 'status',
          state: 'error',
          message: `Invalid deviceId: ${msg.deviceId}`,
        });
        return;
      }
      abortController?.abort();
      abortController = new AbortController();
      paused = false;
      frameSeq = 0;
      startStream(msg.deviceId, msg.apiBase).catch((err: unknown) => {
        const e = err as Error;
        // 主动停止/切换设备导致的 AbortError 视为正常取消，不报错误
        if (e.name === 'AbortError') {
          post({ type: 'status', state: 'stopped' });
          return;
        }
        post({ type: 'error', message: `stream failed: ${String(err)}` });
      });
      break;
    }
    case 'stop':
      abortController?.abort();
      abortController = null;
      paused = false;
      post({ type: 'status', state: 'stopped' });
      break;
    case 'pause':
      paused = true;
      break;
    case 'resume':
      paused = false;
      break;
  }
});

function post(msg: WorkerMessage) {
  (self as unknown as Worker).postMessage(msg);
}

async function startStream(deviceId: string, apiBase: string) {
  const url = `${apiBase}/api/stream/${encodeURIComponent(deviceId)}`;
  post({ type: 'status', state: 'connecting' });

  const res = await fetch(url, {
    method: 'GET',
    signal: abortController!.signal,
    headers: {
      Accept: 'multipart/x-mixed-replace, image/jpeg',
    },
  });
  if (!res.ok || !res.body) {
    post({ type: 'error', message: `HTTP ${res.status}` });
    return;
  }

  const contentType = res.headers.get('Content-Type') ?? '';
  const boundary = extractBoundary(contentType);
  const initialLatency = parseInt(res.headers.get('X-Latency-Ms') ?? '0', 10) || 0;
  if (initialLatency > 0) {
    post({ type: 'latency', latencyMs: initialLatency });
  }

  post({ type: 'status', state: 'streaming' });

  const reader = res.body.getReader();
  let buffer = new Uint8Array(0);
  let lastLatency = initialLatency;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (!value || value.length === 0) continue;

    // 追加到 buffer
    const merged = new Uint8Array(buffer.length + value.length);
    merged.set(buffer, 0);
    merged.set(value, buffer.length);
    buffer = merged;

    // 反复抽取完整帧
    while (true) {
      const result = boundary
        ? extractMultipartFrame(buffer, boundary)
        : extractRawJpegFrame(buffer);
      if (!result) break;

      const { frame, endIdx, latencyMs } = result;
      const effectiveLatency = latencyMs ?? lastLatency;
      if (latencyMs != null) lastLatency = latencyMs;

      if (!paused) {
        // 复制到 ArrayBuffer 拷贝（避免 TS 5.7 Uint8Array<ArrayBufferLike> 与 BlobPart 不兼容）
        const buf = new Uint8Array(frame.byteLength);
        buf.set(frame);
        const blob = new Blob([buf], { type: 'image/jpeg' });
        frameSeq += 1;
        const msg: WorkerMessage = {
          type: 'frame',
          blob,
          latencyMs: effectiveLatency,
          frameSeq,
        };
        // Blob 不可转移，使用结构化克隆传递
        (self as unknown as Worker).postMessage(msg);
      }
      buffer = buffer.slice(endIdx);
    }

    // 防止 buffer 无限增长（找不到 SOI 时丢弃头部）
    if (buffer.length > MAX_BUFFER) {
      const soi = findSoi(buffer);
      if (soi > 0) {
        buffer = buffer.slice(soi);
      } else if (soi === -1) {
        buffer = new Uint8Array(0);
      }
    }
  }

  post({ type: 'status', state: 'stopped' });
}

/** 从 Content-Type 解析 multipart boundary */
function extractBoundary(contentType: string): string | null {
  const m = /boundary=("?)([^";\s]+)\1/i.exec(contentType);
  return m ? `--${m[2]}` : null;
}

/** 在 haystack 中查找 needle（字节序列） */
function indexOfBytes(haystack: Uint8Array, needle: Uint8Array, from: number): number {
  if (needle.length === 0) return -1;
  outer: for (let i = from; i <= haystack.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

/** 查找 JPEG SOI (0xFFD8) */
function findSoi(buf: Uint8Array, from = 0): number {
  for (let i = from; i < buf.length - 1; i++) {
    if (buf[i] === 0xff && buf[i + 1] === 0xd8) return i;
  }
  return -1;
}

/** 查找 JPEG EOI (0xFFD9) */
function findEoi(buf: Uint8Array, from: number): number {
  for (let i = from; i < buf.length - 1; i++) {
    if (buf[i] === 0xff && buf[i + 1] === 0xd9) return i + 2;
  }
  return -1;
}

/** 裸 JPEG 字节流：扫描 SOI / EOI 切出完整帧 */
function extractRawJpegFrame(
  buf: Uint8Array,
): { frame: Uint8Array; endIdx: number; latencyMs?: number } | null {
  const soi = findSoi(buf);
  if (soi === -1) return null;
  const eoi = findEoi(buf, soi + 2);
  if (eoi === -1) return null;
  return { frame: buf.slice(soi, eoi), endIdx: eoi };
}

/** multipart/x-mixed-replace：按 boundary 切片并解析 part 头 */
function extractMultipartFrame(
  buf: Uint8Array,
  boundary: string,
): { frame: Uint8Array; endIdx: number; latencyMs?: number } | null {
  const bBytes = new TextEncoder().encode(boundary);
  const startIdx = indexOfBytes(buf, bBytes, 0);
  if (startIdx === -1) return null;

  const nextIdx = indexOfBytes(buf, bBytes, startIdx + bBytes.length);
  if (nextIdx === -1) {
    // 还没拿到完整 part，等更多数据
    return null;
  }

  // part 内容 = boundary 后到下个 boundary 前
  // 头部 \r\n 开头，body 与 boundary 之间 \r\n
  const partStart = startIdx + bBytes.length;
  const part = buf.subarray(partStart, nextIdx);

  // 找头部终止符 \r\n\r\n
  const headerEnd = findSubarray(part, new TextEncoder().encode('\r\n\r\n'));
  if (headerEnd === -1) {
    return null; // 头部不全
  }

  const headerBytes = part.subarray(0, headerEnd);
  // body 起始 = headerEnd + 4，结尾 -2（去掉 \r\n）
  const bodyStart = headerEnd + 4;
  const bodyEnd = part.length - 2;
  if (bodyEnd <= bodyStart) return null;

  const headerStr = new TextDecoder().decode(headerBytes);
  const latencyMatch = /X-Latency-Ms:\s*(\d+)/i.exec(headerStr);
  const latencyMs = latencyMatch ? parseInt(latencyMatch[1], 10) : undefined;

  const partBody = part.subarray(bodyStart, bodyEnd);
  // 进一步按 JPEG SOI/EOI 截断（防止尾部 padding）
  const soi = findSoi(partBody);
  if (soi === -1) return null;
  const eoi = findEoi(partBody, soi + 2);
  if (eoi === -1) return null;

  return {
    frame: partBody.slice(soi, eoi),
    endIdx: nextIdx,
    latencyMs,
  };
}

function findSubarray(haystack: Uint8Array, needle: Uint8Array): number {
  return indexOfBytes(haystack, needle, 0);
}

export {};
