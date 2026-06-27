<script setup lang="ts">
// 通用分节容器：标题 + 提示 + 主体（默认插槽）
defineProps<{
  title: string;
  hint?: string;
}>();
</script>

<template>
  <section class="px-4 py-3 border-b border-border">
    <div class="flex items-baseline justify-between mb-2">
      <h3 class="font-mono text-[9px] uppercase tracking-[0.25em] text-ink-faint">{{ title }}</h3>
      <span v-if="hint" class="font-mono text-[9px] uppercase tracking-wider text-ink-faint/70">
        {{ hint }}
      </span>
    </div>
    <slot />
  </section>
</template>
