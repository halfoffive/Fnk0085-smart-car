<script setup lang="ts">
// 设备选择下拉框：显示 deviceId + 在线状态，切换时回调
import { computed } from 'vue';
import type { Device } from '../types/protocol';

const props = defineProps<{
  devices: Device[];
  selected: string | null;
  loading: boolean;
  error: string | null;
}>();

const emit = defineEmits<{
  (e: 'select', deviceId: string): void;
  (e: 'refresh'): void;
}>();

// 在线优先，其次 deviceId 字典序
const sorted = computed(() =>
  [...props.devices].sort((a, b) => {
    if (a.online !== b.online) return a.online ? -1 : 1;
    return a.deviceId.localeCompare(b.deviceId);
  }),
);

const selectedDevice = computed(
  () => props.devices.find((d) => d.deviceId === props.selected) ?? null,
);

function formatLastSeen(ts: number): string {
  if (!ts) return '—';
  const diff = Date.now() - ts;
  if (diff < 1000) return 'just now';
  if (diff < 60_000) return `${Math.round(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.round(diff / 60_000)}m ago`;
  return new Date(ts).toLocaleTimeString('zh-CN', { hour12: false });
}
</script>

<template>
  <div class="space-y-2">
    <div class="flex items-center justify-between">
      <label class="font-mono text-[10px] uppercase tracking-[0.2em] text-ink-faint">
        device ▸ select
      </label>
      <button
        type="button"
        @click="emit('refresh')"
        class="font-mono text-[10px] uppercase tracking-wider text-ink-dim hover:text-accent transition-colors"
        title="刷新设备列表"
      >
        {{ loading ? '⟳ scanning…' : '⟳ refresh' }}
      </button>
    </div>

    <div class="relative">
      <select
        :value="selected ?? ''"
        @change="(e) => emit('select', (e.target as HTMLSelectElement).value)"
        class="w-full appearance-none bg-surface-2 border border-border-bright hover:border-accent/60 focus:border-accent focus:outline-none px-3 py-2.5 pr-9 font-mono text-sm text-ink transition-colors"
      >
        <option value="" disabled>
          {{ loading ? '扫描设备中…' : '— 选择设备 —' }}
        </option>
        <option v-for="d in sorted" :key="d.deviceId" :value="d.deviceId">
          {{ d.deviceId }}
        </option>
      </select>
      <!-- 自定义箭头 -->
      <svg
        class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2"
        width="10"
        height="10"
        viewBox="0 0 10 10"
        fill="none"
      >
        <path d="M2 4 L5 7 L8 4" stroke="currentColor" stroke-width="1.5" />
      </svg>
    </div>

    <!-- 选中设备状态条 -->
    <div
      v-if="selectedDevice"
      class="flex items-center gap-2 px-2 py-1.5 bg-surface border-l-2 border-accent"
    >
      <span
        :class="[
          'inline-block w-1.5 h-1.5 rounded-full',
          selectedDevice.online ? 'bg-accent pulse-dot' : 'bg-ink-faint',
        ]"
      />
      <span class="font-mono text-[10px] uppercase tracking-wider text-ink-dim">
        {{ selectedDevice.online ? 'live' : 'offline' }}
      </span>
      <span class="font-mono text-[10px] text-ink-faint ml-auto">
        last seen {{ formatLastSeen(selectedDevice.lastSeenMs) }}
      </span>
    </div>

    <div
      v-if="error"
      class="px-2 py-1.5 bg-danger/10 border-l-2 border-danger font-mono text-[10px] text-danger"
    >
      {{ error }}
    </div>

    <div
      v-if="devices.length === 0 && !loading && !error"
      class="px-2 py-2 bg-surface border border-dashed border-border-bright font-mono text-[10px] text-ink-faint text-center"
    >
      no devices registered
    </div>
  </div>
</template>
