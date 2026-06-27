// 视频流显示组件：Worker 推帧 → createObjectURL 渲染 + 延时/状态显示

import { useEffect, useRef, useState } from 'react';
import type { WorkerCommand, WorkerMessage } from '../types/protocol';
import { API_BASE } from '../lib/constants';

interface VideoStreamProps {
  deviceId: string | null;
  paused: boolean;
  /** 拍照期间外部主动暂停 */
  photoPending?: boolean;
}

type StreamState = 'idle' | 'connecting' | 'streaming' | 'stopped' | 'error';

export function VideoStream({ deviceId, paused, photoPending }: VideoStreamProps) {
  const imgRef = useRef<HTMLImageElement | null>(null);
  const objectUrlRef = useRef<string | null>(null);
  const workerRef = useRef<Worker | null>(null);
  const [state, setState] = useState<StreamState>('idle');
  const [latencyMs, setLatencyMs] = useState<number>(0);
  const [frameSeq, setFrameSeq] = useState<number>(0);
  const [errorMsg, setErrorMsg] = useState<string>('');
  const [fps, setFps] = useState<number>(0);

  // FPS 计算
  const fpsCounterRef = useRef<{ count: number; ts: number }>({ count: 0, ts: Date.now() });

  // 启动 / 切换设备 → 创建 Worker
  useEffect(() => {
    if (!deviceId) {
      setState('idle');
      return;
    }
    setState('connecting');
    setErrorMsg('');

    const worker = new Worker(new URL('../workers/videoWorker.ts', import.meta.url), {
      type: 'module',
    });
    workerRef.current = worker;

    worker.addEventListener('message', (e: MessageEvent<WorkerMessage>) => {
      const msg = e.data;
      switch (msg.type) {
        case 'frame': {
          // 释放上一帧 URL
          if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
          const url = URL.createObjectURL(msg.blob);
          objectUrlRef.current = url;
          if (imgRef.current) imgRef.current.src = url;
          setLatencyMs(msg.latencyMs);
          setFrameSeq(msg.frameSeq);
          setState('streaming');
          // FPS
          const counter = fpsCounterRef.current;
          counter.count += 1;
          const now = Date.now();
          if (now - counter.ts >= 1000) {
            setFps(counter.count);
            counter.count = 0;
            counter.ts = now;
          }
          break;
        }
        case 'latency':
          setLatencyMs(msg.latencyMs);
          break;
        case 'status':
          setState(msg.state);
          if (msg.state === 'error') setErrorMsg(msg.message ?? 'unknown error');
          break;
        case 'error':
          setState('error');
          setErrorMsg(msg.message);
          break;
      }
    });

    const cmd: WorkerCommand = { type: 'start', deviceId, apiBase: API_BASE };
    worker.postMessage(cmd);

    return () => {
      worker.postMessage({ type: 'stop' } satisfies WorkerCommand);
      worker.terminate();
      if (objectUrlRef.current) {
        URL.revokeObjectURL(objectUrlRef.current);
        objectUrlRef.current = null;
      }
    };
  }, [deviceId]);

  // 暂停 / 恢复
  useEffect(() => {
    const worker = workerRef.current;
    if (!worker) return;
    worker.postMessage({ type: paused ? 'pause' : 'resume' } satisfies WorkerCommand);
  }, [paused]);

  // 拍照期间主动暂停
  useEffect(() => {
    const worker = workerRef.current;
    if (!worker) return;
    worker.postMessage({ type: photoPending ? 'pause' : 'resume' } satisfies WorkerCommand);
  }, [photoPending]);

  const showSkeleton =
    state === 'connecting' || state === 'idle' || (state === 'streaming' && frameSeq === 0);
  const showError = state === 'error';
  const showOffline = deviceId === null;

  return (
    <div className="relative h-full w-full bg-black overflow-hidden corner-crosshair scanline">
      {/* 顶部状态条 */}
      <div className="absolute top-0 left-0 right-0 z-20 flex items-center justify-between px-4 py-2 bg-gradient-to-b from-black/80 to-transparent">
        <div className="flex items-center gap-3 font-mono text-[10px] uppercase tracking-wider">
          <span className="flex items-center gap-1.5">
            <span
              className={`inline-block w-1.5 h-1.5 rounded-full ${
                state === 'streaming'
                  ? 'bg-accent pulse-dot'
                  : state === 'connecting'
                    ? 'bg-amber'
                    : state === 'error'
                      ? 'bg-danger'
                      : 'bg-ink-faint'
              }`}
            />
            <span className={state === 'streaming' ? 'text-accent' : 'text-ink-dim'}>
              {state === 'streaming' ? 'live' : state}
            </span>
          </span>
          <span className="text-ink-faint">·</span>
          <span className="text-ink-dim">
            seq <span className="text-ink digit-flicker">{frameSeq.toString().padStart(6, '0')}</span>
          </span>
          <span className="text-ink-faint">·</span>
          <span className="text-ink-dim">
            fps <span className="text-ink">{fps}</span>
          </span>
        </div>
        <div className="flex items-center gap-3 font-mono text-[10px] uppercase tracking-wider">
          <span className="text-ink-faint">latency</span>
          <span
            className={`px-2 py-0.5 ${
              latencyMs <= 100
                ? 'text-accent border border-accent/40'
                : latencyMs <= 200
                  ? 'text-amber border border-amber/40'
                  : 'text-danger border border-danger/40'
            }`}
          >
            {latencyMs} ms
          </span>
        </div>
      </div>

      {/* 视频帧 */}
      <img
        ref={imgRef}
        alt=""
        className={`absolute inset-0 w-full h-full object-contain transition-opacity duration-150 ${
          showSkeleton || showOffline || showError ? 'opacity-0' : 'opacity-100'
        }`}
        draggable={false}
      />

      {/* 骨架屏 */}
      {showSkeleton && !showOffline && !showError && (
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <div className="absolute inset-0 skeleton bg-gradient-to-br from-surface via-surface-2 to-elevated" />
          <div className="relative z-10 flex flex-col items-center gap-3">
            <div className="w-10 h-10 conic-spinner rounded-full" />
            <div className="font-mono text-[10px] uppercase tracking-[0.3em] text-ink-dim">
              {deviceId ? `connecting to ${deviceId}` : 'initializing'}
            </div>
          </div>
        </div>
      )}

      {/* 离线提示 */}
      {showOffline && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.3em] text-ink-faint">
            no device selected
          </div>
          <div className="font-mono text-[10px] text-ink-faint/60">
            ← 从右侧选择一台在线设备
          </div>
        </div>
      )}

      {/* 错误提示 */}
      {showError && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.3em] text-danger">
            stream error
          </div>
          <div className="font-mono text-[11px] text-ink-dim max-w-md text-center px-4">
            {errorMsg}
          </div>
        </div>
      )}

      {/* 拍照遮罩 */}
      {photoPending && (
        <div className="absolute inset-0 z-30 flex flex-col items-center justify-center bg-black/70 backdrop-blur-sm">
          <div className="w-16 h-16 conic-spinner rounded-full mb-4" />
          <div className="font-mono text-xs uppercase tracking-[0.3em] text-amber">
            capturing photo
          </div>
          <div className="font-mono text-[10px] text-ink-faint mt-2">
            视频流已暂停，等待设备回执…
          </div>
        </div>
      )}

      {/* 底部信息条 */}
      <div className="absolute bottom-0 left-0 right-0 z-20 flex items-center justify-between px-4 py-1.5 bg-gradient-to-t from-black/80 to-transparent font-mono text-[9px] uppercase tracking-wider text-ink-faint">
        <span>QVGA · 320×240 · JPEG</span>
        <span>{deviceId ?? '—'}</span>
      </div>
    </div>
  );
}
