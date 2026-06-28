<script setup lang="ts">
// 视频流显示组件：Worker 推帧 → createObjectURL 渲染 + 延时/状态显示
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import type { WorkerCommand, WorkerMessage } from '../types/protocol';
import { API_BASE } from '../lib/constants';

const props = defineProps<{
  deviceId: string | null;
  paused: boolean;
  /** 拍照期间外部主动暂停 */
  photoPending?: boolean;
}>();

type StreamState = 'idle' | 'connecting' | 'streaming' | 'stopped' | 'error';

const imgRef = ref<HTMLImageElement | null>(null);
const src = ref<string>('');
const state = ref<StreamState>('idle');
const latencyMs = ref<number>(0);
const frameSeq = ref<number>(0);
const errorMsg = ref<string>('');
const fps = ref<number>(0);

// Worker / objectUrl 用 shallowRef 持有，避免深层响应式
const workerRef = shallowRef<Worker | null>(null);
let objectUrl: string | null = null;

// FPS 计算
let fpsCount = 0;
let fpsTs = Date.now();

function onMessage(e: MessageEvent<WorkerMessage>) {
  const msg = e.data;
  switch (msg.type) {
    case 'frame': {
      // 释放上一帧 URL
      if (objectUrl) URL.revokeObjectURL(objectUrl);
      const url = URL.createObjectURL(msg.blob);
      objectUrl = url;
      src.value = url;
      latencyMs.value = msg.latencyMs;
      frameSeq.value = msg.frameSeq;
      state.value = 'streaming';
      // FPS
      fpsCount += 1;
      const now = Date.now();
      if (now - fpsTs >= 1000) {
        fps.value = fpsCount;
        fpsCount = 0;
        fpsTs = now;
      }
      break;
    }
    case 'latency':
      latencyMs.value = msg.latencyMs;
      break;
    case 'status':
      state.value = msg.state;
      if (msg.state === 'error') errorMsg.value = msg.message ?? 'unknown error';
      break;
    case 'error':
      state.value = 'error';
      errorMsg.value = msg.message;
      break;
  }
}

// 启动 / 切换设备 → 创建 Worker
watch(
  () => props.deviceId,
  (id) => {
    // 清理上一任 worker
    if (workerRef.value) {
      workerRef.value.postMessage({ type: 'stop' } satisfies WorkerCommand);
      workerRef.value.terminate();
      workerRef.value = null;
    }
    if (objectUrl) {
      URL.revokeObjectURL(objectUrl);
      objectUrl = null;
    }
    src.value = '';

    if (!id) {
      state.value = 'idle';
      return;
    }
    state.value = 'connecting';
    errorMsg.value = '';

    const worker = new Worker(new URL('../workers/videoWorker.ts', import.meta.url), {
      type: 'module',
    });
    workerRef.value = worker;
    worker.addEventListener('message', onMessage);

    const cmd: WorkerCommand = { type: 'start', deviceId: id, apiBase: API_BASE };
    worker.postMessage(cmd);
  },
  { immediate: true },
);

// 暂停 / 恢复（普通暂停）
watch(
  () => props.paused,
  (paused) => {
    const worker = workerRef.value;
    if (!worker) return;
    worker.postMessage({ type: paused ? 'pause' : 'resume' } satisfies WorkerCommand);
  },
);

// 拍照期间主动暂停
watch(
  () => props.photoPending,
  (pending) => {
    const worker = workerRef.value;
    if (!worker) return;
    worker.postMessage({ type: pending ? 'pause' : 'resume' } satisfies WorkerCommand);
  },
);

onBeforeUnmount(() => {
  if (workerRef.value) {
    workerRef.value.postMessage({ type: 'stop' } satisfies WorkerCommand);
    workerRef.value.terminate();
    workerRef.value = null;
  }
  if (objectUrl) {
    URL.revokeObjectURL(objectUrl);
    objectUrl = null;
  }
  src.value = '';
});

const showSkeleton = computed(
  () => state.value === 'connecting' || state.value === 'idle' || (state.value === 'streaming' && frameSeq.value === 0),
);
const showError = computed(() => state.value === 'error');
const showOffline = computed(() => props.deviceId === null);
const stateColor = computed(() => {
  if (state.value === 'streaming') return 'bg-accent pulse-dot';
  if (state.value === 'connecting') return 'bg-amber';
  if (state.value === 'error') return 'bg-danger';
  return 'bg-ink-faint';
});
const stateText = computed(() => (state.value === 'streaming' ? 'live' : state.value));
const latencyClass = computed(() => {
  if (latencyMs.value <= 100) return 'text-accent border border-accent/40';
  if (latencyMs.value <= 200) return 'text-amber border border-amber/40';
  return 'text-danger border border-danger/40';
});
const frameSeqText = computed(() => frameSeq.value.toString().padStart(6, '0'));
</script>

<template>
  <div class="relative h-full w-full bg-black overflow-hidden corner-crosshair scanline">
    <!-- 顶部状态条 -->
    <div
      class="absolute top-0 left-0 right-0 z-20 flex items-center justify-between px-4 py-2 bg-gradient-to-b from-black/80 to-transparent"
    >
      <div class="flex items-center gap-3 font-mono text-[10px] uppercase tracking-wider">
        <span class="flex items-center gap-1.5">
          <span :class="['inline-block w-1.5 h-1.5 rounded-full', stateColor]" />
          <span :class="state === 'streaming' ? 'text-accent' : 'text-ink-dim'">{{ stateText }}</span>
        </span>
        <span class="text-ink-faint">·</span>
        <span class="text-ink-dim">
          seq <span class="text-ink digit-flicker">{{ frameSeqText }}</span>
        </span>
        <span class="text-ink-faint">·</span>
        <span class="text-ink-dim">
          fps <span class="text-ink">{{ fps }}</span>
        </span>
      </div>
      <div class="flex items-center gap-3 font-mono text-[10px] uppercase tracking-wider">
        <span class="text-ink-faint">latency</span>
        <span :class="['px-2 py-0.5', latencyClass]">{{ latencyMs }} ms</span>
      </div>
    </div>

    <!-- 视频帧 -->
    <img
      ref="imgRef"
      :src="src"
      alt=""
      :class="[
        'absolute inset-0 w-full h-full object-contain transition-opacity duration-150',
        showSkeleton || showOffline || showError ? 'opacity-0' : 'opacity-100',
      ]"
      draggable="false"
    />

    <!-- 骨架屏 -->
    <div
      v-if="showSkeleton && !showOffline && !showError"
      class="absolute inset-0 flex flex-col items-center justify-center"
    >
      <div
        class="absolute inset-0 skeleton bg-gradient-to-br from-surface via-surface-2 to-elevated"
      />
      <div class="relative z-10 flex flex-col items-center gap-3">
        <div class="w-10 h-10 conic-spinner rounded-full" />
        <div class="font-mono text-[10px] uppercase tracking-[0.3em] text-ink-dim">
          {{ deviceId ? `connecting to ${deviceId}` : 'initializing' }}
        </div>
      </div>
    </div>

    <!-- 离线提示 -->
    <div v-if="showOffline" class="absolute inset-0 flex flex-col items-center justify-center gap-3">
      <div class="font-mono text-[10px] uppercase tracking-[0.3em] text-ink-faint">
        no device selected
      </div>
      <div class="font-mono text-[10px] text-ink-faint/60">← 从右侧选择一台在线设备</div>
    </div>

    <!-- 错误提示 -->
    <div v-if="showError" class="absolute inset-0 flex flex-col items-center justify-center gap-3">
      <div class="font-mono text-[10px] uppercase tracking-[0.3em] text-danger">stream error</div>
      <div class="font-mono text-[11px] text-ink-dim max-w-md text-center px-4">
        {{ errorMsg }}
      </div>
    </div>

    <!-- 拍照遮罩 -->
    <div
      v-if="photoPending"
      class="absolute inset-0 z-30 flex flex-col items-center justify-center bg-black/70 backdrop-blur-sm"
    >
      <div class="w-16 h-16 conic-spinner rounded-full mb-4" />
      <div class="font-mono text-xs uppercase tracking-[0.3em] text-amber">capturing photo</div>
      <div class="font-mono text-[10px] text-ink-faint mt-2">视频流已暂停，等待设备回执…</div>
    </div>

    <!-- 底部信息条 -->
    <div
      class="absolute bottom-0 left-0 right-0 z-20 flex items-center justify-between px-4 py-1.5 bg-gradient-to-t from-black/80 to-transparent font-mono text-[9px] uppercase tracking-wider text-ink-faint"
    >
      <span>QVGA · 320×240 · JPEG</span>
      <span>{{ deviceId ?? '—' }}</span>
    </div>
  </div>
</template>
