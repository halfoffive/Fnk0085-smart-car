<script setup lang="ts">
// 主应用：左侧视频流 + 右侧控制面板 + 顶部状态条 + Toast + 配网弹窗 + PWA 注册
import { computed, onMounted, ref, watch } from 'vue';
import DeviceSelect from './components/DeviceSelect.vue';
import VideoStream from './components/VideoStream.vue';
import ControlPanel from './components/ControlPanel.vue';
import ConfigDialog from './components/ConfigDialog.vue';
import LoginView from './components/LoginView.vue';
import { useKeyboard } from './composables/useKeyboard';
import { useDevices } from './composables/useDevices';
import { postControl, pwmToSlider, sliderToPwm } from './lib/api';
import type { Direction } from './types/protocol';
import { isAuthed, clearAuthed } from './lib/auth';
import { APP_VERSION, PWM_MAX } from './lib/constants';
import { registerSW } from 'virtual:pwa-register';

type ToastKind = 'info' | 'success' | 'error';
interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

const { devices, loading, error, refresh } = useDevices();
const selectedDevice = ref<string | null>(null);
const pwm = ref<number>(sliderToPwm(50));
const configOpen = ref(false);
const photoPending = ref(false);
const keyboardActiveDir = ref<Direction | null>(null);
const toasts = ref<Toast[]>([]);
let toastId = 0;

// 访问密码登录态（sessionStorage 生命周期 = tab）
const authed = ref(isAuthed());

function handleLogout() {
  clearAuthed();
  authed.value = false;
}

// PWA SW 注册：版本更新提示
onMounted(() => {
  if (import.meta.env.PROD) {
    registerSW({
      onRegistered: (reg) => {
        // eslint-disable-next-line no-console
        console.info('[PWA] service worker registered', reg?.scope);
      },
      onRegisterError: (err) => {
        console.warn('[PWA] sw register failed', err);
      },
    });
  }
});

// 默认选第一个在线设备
watch(
  [devices, selectedDevice],
  ([list, sel]) => {
    if (sel) {
      const stillExists = list.some((d) => d.deviceId === sel);
      if (stillExists) return;
    }
    const firstOnline = list.find((d) => d.online);
    if (firstOnline) {
      selectedDevice.value = firstOnline.deviceId;
    } else if (list.length > 0) {
      selectedDevice.value = null;
    }
  },
  { flush: 'post' },
);

function pushToast(kind: ToastKind, message: string) {
  const id = ++toastId;
  toasts.value = [...toasts.value, { id, kind, message }];
  setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }, 4000);
}

function handlePhotoResult(path: string) {
  pushToast('success', `已拍照：${path}`);
}
function handlePhotoError(msg: string) {
  pushToast('error', `拍照失败：${msg}`);
}

// 滑块 / PWM 统一更新入口
function handlePwmChange(v: number) {
  pwm.value = v;
}

// 方向键 ±5（slider 单位）调速，再映射回 PWM 0-255
function handleSpeedDelta(delta: number) {
  const nextSlider = Math.max(1, Math.min(100, pwmToSlider(pwm.value) + delta));
  handlePwmChange(sliderToPwm(nextSlider));
}

// 全局键盘：WASD 遥控 + 上下箭头调速，仅在选中设备时生效
const keyboardInFlight = new Set<Direction>();
async function handleKeyPress(dir: Direction) {
  const id = selectedDevice.value;
  if (!id || keyboardInFlight.has(dir)) return;
  keyboardActiveDir.value = dir;
  keyboardInFlight.add(dir);
  try {
    await postControl(id, { direction: dir, pwm: pwm.value });
  } catch {
    // 静默失败
  } finally {
    keyboardInFlight.delete(dir);
  }
}
async function handleKeyRelease(_dir: Direction) {
  const id = selectedDevice.value;
  if (!id) return;
  keyboardActiveDir.value = null;
  try {
    await postControl(id, { direction: 'stop', pwm: 0 });
  } catch {
    // ignore
  }
}

useKeyboard({
  onPress: handleKeyPress,
  onRelease: handleKeyRelease,
  enabled: () => !!selectedDevice.value,
  onSpeedDelta: handleSpeedDelta,
});

const onlineCount = computed(() => devices.value.filter((d) => d.online).length);
const selectedDeviceInfo = computed(() =>
  devices.value.find((d) => d.deviceId === selectedDevice.value),
);
const toastClass = (kind: ToastKind) =>
  kind === 'success'
    ? 'bg-accent/10 border-accent text-accent'
    : kind === 'error'
      ? 'bg-danger/10 border-danger text-danger'
      : 'bg-surface/90 border-border-bright text-ink';
const toastIcon = (kind: ToastKind) => (kind === 'success' ? '✓' : kind === 'error' ? '✕' : 'ℹ');
</script>

<template>
  <LoginView v-if="!authed" @authed="authed = true" />
  <div v-else class="h-screen w-screen flex flex-col bg-base text-ink overflow-hidden">
    <!-- ============ 顶部状态条 ============ -->
    <header
      class="flex items-center justify-between px-4 h-12 border-b border-border bg-surface/80 backdrop-blur z-30"
    >
      <!-- 左：品牌 -->
      <div class="flex items-center gap-3">
        <div class="flex items-center gap-2">
          <span class="w-2 h-2 bg-accent pulse-dot" />
          <span class="font-mono text-sm font-bold tracking-tight">
            Fnk<span class="text-accent">0085</span>
          </span>
        </div>
        <span class="font-mono text-[9px] uppercase tracking-[0.25em] text-ink-faint">
          smart car console
        </span>
      </div>

      <!-- 中：导航 -->
      <nav class="flex items-center gap-4 font-mono text-[10px] uppercase tracking-wider text-ink-dim">
        <span class="flex items-center gap-1.5">
          <span class="w-1 h-1 bg-accent" />
          <span>console</span>
        </span>
        <span class="text-ink-faint">/</span>
        <span class="text-ink-faint">fleet</span>
        <span class="text-ink-faint">/</span>
        <span class="text-ink-faint">telemetry</span>
      </nav>

      <!-- 右：状态 -->
      <div class="flex items-center gap-4 font-mono text-[10px] uppercase tracking-wider">
        <span class="flex items-center gap-1.5">
          <span class="text-ink-faint">devices</span>
          <span class="text-ink">{{ onlineCount }}/{{ devices.length }}</span>
        </span>
        <span class="text-ink-faint">·</span>
        <span class="text-ink-dim">
          v<span class="text-ink">{{ APP_VERSION }}</span>
        </span>
        <span class="text-ink-faint">·</span>
        <button
          type="button"
          class="text-ink-faint hover:text-ink transition-colors font-mono text-[10px] uppercase tracking-wider"
          @click="handleLogout"
        >
          logout
        </button>
      </div>
    </header>

    <!-- ============ 主区域 ============ -->
    <main class="flex-1 flex min-h-0">
      <!-- 左侧：视频流 + 设备状态条 -->
      <div class="flex-1 flex flex-col min-w-0 border-r border-border">
        <!-- 设备状态条 -->
        <div
          class="flex items-center gap-3 px-4 h-9 bg-surface-2 border-b border-border font-mono text-[10px] uppercase tracking-wider"
        >
          <span class="text-ink-faint">device ▸</span>
          <span class="truncate-mono text-ink max-w-md">
            {{ selectedDevice ?? '— no device —' }}
          </span>
          <div class="ml-auto flex items-center gap-3">
            <span class="text-ink-faint">
              online:
              <span class="text-accent">{{ onlineCount }}</span>
            </span>
            <span class="text-ink-faint">·</span>
            <span class="text-ink-faint">
              pwm:
              <span class="text-amber">{{ pwm }}/{{ PWM_MAX }}</span>
            </span>
            <span class="text-ink-faint">·</span>
            <span class="text-ink-faint">
              rpm:
              <span class="text-accent">{{ selectedDeviceInfo?.leftRpm ?? 0 }}</span>
              /
              <span class="text-accent">{{ selectedDeviceInfo?.rightRpm ?? 0 }}</span>
            </span>
          </div>
        </div>

        <!-- 视频流 -->
        <div class="flex-1 min-h-0 relative">
          <VideoStream :device-id="selectedDevice" :paused="false" :photo-pending="photoPending" />
        </div>
      </div>

      <!-- 右侧：控制面板 -->
      <aside class="w-[340px] flex flex-col bg-surface min-h-0">
        <!-- 设备选择 -->
        <div class="px-4 py-3 border-b border-border">
          <DeviceSelect
            :devices="devices"
            :selected="selectedDevice"
            :loading="loading"
            :error="error"
            @select="(id) => (selectedDevice = id)"
            @refresh="refresh"
          />
        </div>

        <!-- 控制面板 -->
        <div class="flex-1 min-h-0">
          <ControlPanel
            :device-id="selectedDevice"
            :pwm="pwm"
            :keyboard-active-dir="keyboardActiveDir"
            @pwm-change="handlePwmChange"
            @open-config="configOpen = true"
            @photo-before="photoPending = true"
            @photo-after="photoPending = false"
            @photo-result="handlePhotoResult"
            @photo-error="handlePhotoError"
          />
        </div>

        <!-- 底部信息条 -->
        <footer
          class="px-4 py-2 border-t border-border font-mono text-[8px] uppercase tracking-wider text-ink-faint flex items-center justify-between"
        >
          <span>udp+tls · dtls · mjpeg</span>
          <span class="flex items-center gap-1">
            <span class="w-1 h-1 bg-accent rounded-full" />
            <span>system nominal</span>
          </span>
        </footer>
      </aside>
    </main>

    <!-- ============ Toast ============ -->
    <div class="fixed top-14 right-4 z-50 flex flex-col gap-2 pointer-events-none">
      <div
        v-for="t in toasts"
        :key="t.id"
        :class="[
          'toast-in pointer-events-auto px-3 py-2 min-w-[240px] max-w-sm border-l-2 font-mono text-[11px] backdrop-blur',
          toastClass(t.kind),
        ]"
      >
        <div class="flex items-start gap-2">
          <span class="font-bold">{{ toastIcon(t.kind) }}</span>
          <span class="flex-1 break-words">{{ t.message }}</span>
        </div>
      </div>
    </div>

    <!-- ============ 配网弹窗 ============ -->
    <ConfigDialog :open="configOpen" @close="configOpen = false" />
  </div>
</template>
