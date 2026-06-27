<script setup lang="ts">
// D-pad 方向按钮：pointerdown/up/leave 控制
import { computed } from 'vue';
import type { Direction } from '../types/protocol';

const props = withDefaults(
  defineProps<{
    dir: Direction;
    active: boolean;
    extraClass?: string;
  }>(),
  { extraClass: '' },
);

const emit = defineEmits<{
  (e: 'down', dir: Direction): void;
  (e: 'up'): void;
}>();

const DIR_META: Record<Direction, { label: string; sub: string }> = {
  W: { label: 'W', sub: 'fwd' },
  A: { label: 'A', sub: 'left' },
  S: { label: 'S', sub: 'back' },
  D: { label: 'D', sub: 'right' },
  stop: { label: '■', sub: 'stop' },
};

const meta = computed(() => DIR_META[props.dir]);

function onPointerDown(e: PointerEvent) {
  e.preventDefault();
  emit('down', props.dir);
}
function onPointerUp(e: PointerEvent) {
  e.preventDefault();
  emit('up');
}
function onPointerLeave() {
  if (props.active) emit('up');
}
</script>

<template>
  <button
    type="button"
    @pointerdown="onPointerDown"
    @pointerup="onPointerUp"
    @pointerleave="onPointerLeave"
    @pointercancel="emit('up')"
    :class="[
      'relative flex flex-col items-center justify-center aspect-square bg-surface-2 border transition-all',
      active
        ? 'border-accent bg-accent/10 key-active scale-95'
        : 'border-border-bright hover:border-accent/40',
      extraClass,
    ]"
  >
    <span :class="['font-mono text-lg font-bold', active ? 'text-accent' : 'text-ink']">
      {{ meta.label }}
    </span>
    <span class="font-mono text-[8px] uppercase tracking-wider text-ink-faint">{{ meta.sub }}</span>
    <span v-if="active" class="absolute top-1 right-1 w-1 h-1 bg-accent rounded-full" />
  </button>
</template>
