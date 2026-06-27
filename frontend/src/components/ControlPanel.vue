<script setup lang="ts">
// 控制面板：WASD D-pad + 1-100 调速滑块 + 智能修正开关 + 拍照按钮 + 配网入口 + 轮速显示
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useKeyboard } from '../composables/useKeyboard';
import { getPwmCache, getTelemetry, postControl, postPwmCache, sliderToPwm } from '../lib/api';
import { PWM_DEBOUNCE_MS, PWM_MAX, SLIDER_MAX, SLIDER_MIN } from '../lib/constants';
import type { Direction, TelemetryState } from '../types/protocol';
import PadButton from './PadButton.vue';
import PhotoButton from './PhotoButton.vue';
import Section from './Section.vue';

const props = defineProps<{
  deviceId: string | null;
  /** 当前 PWM 值（受控） */
  pwm: number;
}>();

const emit = defineEmits<{
  (e: 'pwmChange', pwm: number): void;
  (e: 'openConfig'): void;
  (e: 'photoBefore'): void;
  (e: 'photoAfter'): void;
  (e: 'photoResult', path: string): void;
  (e: 'photoError', msg: string): void;
}>();

const activeDir = ref<Direction | null>(null);
const cacheEnabled = ref<boolean | null>(null);
const cacheLoading = ref(false);
const telemetry = ref<TelemetryState>({ leftRpm: 0, rightRpm: 0 });
let telemetryTimer: ReturnType<typeof setInterval> | null = null;

// WASD 在飞未完成集合，避免重复下发
const inFlight = new Set<Direction>();
// 当前 PWM 的最新值快照（供键盘回调读取）
let lastPwm = props.pwm;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;

watch(
  () => props.pwm,
  (v) => {
    lastPwm = v;
  },
);

// 拉取 PWM 缓存状态 与 轮速
watch(
  () => props.deviceId,
  (id) => {
    if (!id) {
      cacheEnabled.value = null;
      telemetry.value = { leftRpm: 0, rightRpm: 0 };
      if (telemetryTimer) {
        clearInterval(telemetryTimer);
        telemetryTimer = null;
      }
      return;
    }
    let cancelled = false;
    cacheLoading.value = true;
    getPwmCache(id)
      .then((s) => {
        if (!cancelled) cacheEnabled.value = s.enabled;
      })
      .catch(() => {
        if (!cancelled) cacheEnabled.value = false;
      })
      .finally(() => {
        if (!cancelled) cacheLoading.value = false;
      });

    // 立即拉一次轮速，之后每 500ms 刷新
    const refreshTelemetry = () => {
      getTelemetry(id).then((t) => {
        if (!cancelled) telemetry.value = t;
      }).catch(() => {
        // 静默失败，保留旧值
      });
    };
    refreshTelemetry();
    if (telemetryTimer) clearInterval(telemetryTimer);
    telemetryTimer = setInterval(refreshTelemetry, 500);

    return () => {
      cancelled = true;
    };
  },
  { immediate: true },
);

// WASD 按下/释放
const handlePress = async (dir: Direction) => {
  if (!props.deviceId) return;
  if (inFlight.has(dir)) return;
  inFlight.add(dir);
  activeDir.value = dir;
  try {
    await postControl(props.deviceId, { direction: dir, pwm: lastPwm });
  } catch {
    // 静默
  } finally {
    inFlight.delete(dir);
  }
};

const handleRelease = async (dir: Direction) => {
  if (!props.deviceId) return;
  if (activeDir.value === dir) activeDir.value = null;
  try {
    await postControl(props.deviceId, { direction: 'stop', pwm: 0 });
  } catch {
    // ignore
  }
};

useKeyboard({
  onPress: handlePress,
  onRelease: handleRelease,
  enabled: () => !!props.deviceId,
});

// D-pad 按钮
const handlePadDown = (dir: Direction) => {
  void handlePress(dir);
};
const handlePadUp = () => {
  void handleRelease(activeDir.value ?? 'stop');
};

// 滑块 debounce（仅同步显示，WASD 时才携带下发）
const handleSliderChange = (val: number) => {
  const newPwm = sliderToPwm(val);
  emit('pwmChange', newPwm);
  if (debounceTimer) clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    // UI 状态展示；实际下发由 WASD 按下时携带
  }, PWM_DEBOUNCE_MS);
};

// 智能修正（原 PWM 缓存）开关
const handleToggleSmartCorrect = async () => {
  if (!props.deviceId || cacheEnabled.value === null || cacheLoading.value) return;
  const next = !cacheEnabled.value;
  cacheLoading.value = true;
  cacheEnabled.value = next; // 乐观更新
  try {
    await postPwmCache(props.deviceId, { enabled: next });
  } catch {
    cacheEnabled.value = !next; // 回滚
  } finally {
    cacheLoading.value = false;
  }
};

const sliderValue = computed(() => Math.round((props.pwm / PWM_MAX) * 100));
const pct = computed(
  () => ((sliderValue.value - SLIDER_MIN) / (SLIDER_MAX - SLIDER_MIN)) * 100,
);
const smartCorrectText = computed(() => {
  if (cacheLoading.value) return '⟳ updating';
  if (cacheEnabled.value === null) return 'unknown';
  return cacheEnabled.value ? '已开启' : '已关闭';
});

onBeforeUnmount(() => {
  if (debounceTimer) clearTimeout(debounceTimer);
  if (telemetryTimer) clearInterval(telemetryTimer);
});
</script>

<template>
  <div class="flex flex-col h-full overflow-y-auto">
    <!-- ============ WASD D-Pad ============ -->
    <Section title="drive ▸ wasd" hint="键盘 / 触控">
      <div class="grid grid-cols-3 grid-rows-2 gap-1.5 max-w-[220px] mx-auto select-none">
        <!-- W -->
        <PadButton
          dir="W"
          :active="activeDir === 'W'"
          extra-class="col-start-2"
          @down="handlePadDown"
          @up="handlePadUp"
        />
        <!-- A -->
        <PadButton
          dir="A"
          :active="activeDir === 'A'"
          extra-class="col-start-1 row-start-2"
          @down="handlePadDown"
          @up="handlePadUp"
        />
        <!-- S -->
        <PadButton
          dir="S"
          :active="activeDir === 'S'"
          extra-class="col-start-2 row-start-2"
          @down="handlePadDown"
          @up="handlePadUp"
        />
        <!-- D -->
        <PadButton
          dir="D"
          :active="activeDir === 'D'"
          extra-class="col-start-3 row-start-2"
          @down="handlePadDown"
          @up="handlePadUp"
        />
      </div>

      <!-- 状态行 -->
      <div class="flex items-center justify-between mt-2 px-1 font-mono text-[9px] uppercase tracking-wider">
        <span class="text-ink-faint">
          active:
          <span :class="activeDir ? 'text-accent' : 'text-ink-faint'">
            {{ activeDir ?? 'idle' }}
          </span>
        </span>
        <span class="text-ink-faint">
          target:
          <span class="text-ink">{{ deviceId ?? '—' }}</span>
        </span>
      </div>
    </Section>

    <!-- ============ 调速滑块 ============ -->
    <Section title="throttle ▸ pwm" :hint="`${pwm} / ${PWM_MAX}`">
      <div class="space-y-2">
        <input
          type="range"
          :min="SLIDER_MIN"
          :max="SLIDER_MAX"
          step="1"
          :value="sliderValue"
          @input="(e) => handleSliderChange(Number((e.target as HTMLInputElement).value))"
          class="range-track w-full"
          :style="{ '--pct': `${pct}%` }"
          :disabled="!deviceId"
        />
        <div class="flex items-center justify-between font-mono text-[9px] uppercase tracking-wider">
          <span class="text-ink-faint">{{ SLIDER_MIN }}</span>
          <span class="text-ink-dim">
            slider <span class="text-ink">{{ sliderValue }}</span> · pwm
            <span class="text-accent">{{ pwm }}</span>
          </span>
          <span class="text-ink-faint">{{ SLIDER_MAX }}</span>
        </div>
        <!-- PWM 条 -->
        <div class="relative h-1 bg-surface-2 overflow-hidden">
          <div
            class="absolute inset-y-0 left-0 bar-fill transition-[width] duration-150"
            :style="{ width: `${(pwm / PWM_MAX) * 100}%` }"
          />
        </div>
      </div>
    </Section>

    <!-- ============ 拍照 ============ -->
    <Section title="capture ▸ photo">
      <PhotoButton
        :device-id="deviceId"
        @before-capture="emit('photoBefore')"
        @after-capture="emit('photoAfter')"
        @result="(path) => emit('photoResult', path)"
        @error="(msg) => emit('photoError', msg)"
      />
    </Section>

    <!-- ============ 轮速显示 ============ -->
    <Section title="telemetry ▸ rpm" :hint="`${telemetry.leftRpm} / ${telemetry.rightRpm}`">
      <div class="grid grid-cols-2 gap-2">
        <div class="bg-surface-2 border border-border-bright px-3 py-2 text-center">
          <div class="font-mono text-[9px] uppercase tracking-wider text-ink-faint mb-1">LEFT RPM</div>
          <div class="font-mono text-sm text-accent">{{ telemetry.leftRpm }}</div>
        </div>
        <div class="bg-surface-2 border border-border-bright px-3 py-2 text-center">
          <div class="font-mono text-[9px] uppercase tracking-wider text-ink-faint mb-1">RIGHT RPM</div>
          <div class="font-mono text-sm text-accent">{{ telemetry.rightRpm }}</div>
        </div>
      </div>
    </Section>

    <!-- ============ 智能修正开关 ============ -->
    <Section title="tuning ▸ smart correct" :hint="smartCorrectText">
      <div class="flex items-center justify-between bg-surface-2 border border-border-bright px-3 py-2.5">
        <div class="flex flex-col">
          <span class="font-mono text-[11px] text-ink">智能修正</span>
          <span class="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
            PID 收敛结果复用
          </span>
        </div>
        <button
          type="button"
          @click="handleToggleSmartCorrect"
          :disabled="!deviceId || cacheLoading || cacheEnabled === null"
          class="px-3 py-1.5 font-mono text-[10px] uppercase tracking-wider border transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
          :class="
            cacheEnabled
              ? 'bg-danger/10 border-danger text-danger hover:bg-danger/20'
              : 'bg-accent/10 border-accent text-accent hover:bg-accent/20'
          "
        >
          {{ cacheEnabled ? '关闭' : '开启' }}
        </button>
      </div>
    </Section>

    <!-- ============ 配网入口 ============ -->
    <Section title="provision ▸ web serial">
      <button
        type="button"
        @click="emit('openConfig')"
        class="w-full group flex items-center justify-between bg-surface-2 border border-border-bright hover:border-accent/60 px-3 py-2.5 transition-colors"
      >
        <div class="flex flex-col items-start">
          <span class="font-mono text-[11px] text-ink group-hover:text-accent transition-colors">
            打开配网弹窗
          </span>
          <span class="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
            WiFi · server · token
          </span>
        </div>
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          class="text-ink-faint group-hover:text-accent transition-colors"
        >
          <path d="M5 3 L9 7 L5 11" stroke="currentColor" stroke-width="1.5" />
        </svg>
      </button>
    </Section>
  </div>
</template>
