<script setup lang="ts">
// 拍照按钮：点击 → 暂停视频流 → POST /api/photo → 等待回执 → 恢复视频流 → toast
import { computed, ref } from 'vue';
import { postPhoto } from '../lib/api';

const props = defineProps<{
  deviceId: string | null;
}>();

const emit = defineEmits<{
  (e: 'beforeCapture'): void;
  (e: 'afterCapture'): void;
  (e: 'result', path: string): void;
  (e: 'error', msg: string): void;
}>();

const loading = ref(false);
const disabled = computed(() => !props.deviceId || loading.value);

async function handleClick() {
  if (!props.deviceId || loading.value) return;
  loading.value = true;
  emit('beforeCapture');
  try {
    const res = await postPhoto(props.deviceId);
    emit('result', res.path);
  } catch (e) {
    emit('error', (e as Error).message);
  } finally {
    loading.value = false;
    emit('afterCapture');
  }
}
</script>

<template>
  <button
    type="button"
    :disabled="disabled"
    @click="handleClick"
    class="group relative w-full overflow-hidden bg-surface-2 border border-amber/40 hover:border-amber focus:outline-none focus:ring-1 focus:ring-amber disabled:opacity-40 disabled:cursor-not-allowed transition-all"
  >
    <!-- 加载圈 -->
    <div v-if="loading" class="flex items-center justify-center gap-3 py-3.5">
      <div class="w-4 h-4 conic-spinner rounded-full" />
      <span class="font-mono text-xs uppercase tracking-[0.3em] text-amber">capturing…</span>
    </div>
    <div v-else class="flex items-center justify-center gap-3 py-3.5">
      <!-- 相机图标 -->
      <svg width="16" height="14" viewBox="0 0 16 14" fill="none" class="text-amber">
        <rect
          x="0.75"
          y="2.75"
          width="14.5"
          height="9.5"
          rx="1.25"
          stroke="currentColor"
          stroke-width="1.5"
        />
        <path
          d="M5 2.75 L5.5 1.5 L10.5 1.5 L11 2.75"
          stroke="currentColor"
          stroke-width="1.5"
        />
        <circle cx="8" cy="7.5" r="2.5" stroke="currentColor" stroke-width="1.5" />
      </svg>
      <span
        class="font-mono text-xs uppercase tracking-[0.3em] text-ink group-hover:text-amber transition-colors"
      >
        capture photo
      </span>
    </div>
    <!-- 悬停光带 -->
    <div
      v-if="!loading"
      class="absolute inset-0 bg-gradient-to-r from-transparent via-amber/5 to-transparent -translate-x-full group-hover:translate-x-full transition-transform duration-700 ease-out"
    />
  </button>
</template>
