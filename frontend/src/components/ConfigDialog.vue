<script setup lang="ts">
// Web Serial 配网弹窗：表单 → 选串口 → 发送 CONFIG → 等 OK / REBOOT
import { onBeforeUnmount, ref, watch } from 'vue';
import { getProvisioning } from '../lib/api';
import { useWebSerial } from '../composables/useWebSerial';

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
}>();

// 解构使 ref 在模板中自动解包
const { supported, connecting, log, configure } = useWebSerial();

const form = ref({ ssid: '', password: '', server: '', token: '' });
const resultMsg = ref<string | null>(null);
const resultOk = ref<boolean | null>(null);
const provisioningLoading = ref(false);
const provisioningError = ref<string | null>(null);

// 关闭时复位
watch(
  () => props.open,
  (open) => {
    if (!open) {
      resultMsg.value = null;
      resultOk.value = null;
      provisioningError.value = null;
    }
  },
);

// 根据当前浏览器地址构造 server 自动填充值（scheme://host:port）
function currentServerFromLocation(): string {
  const proto = window.location.protocol; // 'http:' 或 'https:'
  if (!proto.startsWith('http')) return '';
  const port = window.location.port || (proto === 'https:' ? '443' : '80');
  return `${proto}//${window.location.hostname}:${port}`;
}

// 打开弹窗时自动填充 server / token
watch(
  () => props.open,
  async (open) => {
    if (!open) return;
    provisioningLoading.value = true;
    provisioningError.value = null;
    try {
      const cfg = await getProvisioning();
      // server 优先使用当前浏览器访问地址，用户可修改
      const currentServer = currentServerFromLocation();
      form.value.server = currentServer || `http://${cfg.server}`;
      form.value.token = cfg.token;
    } catch (e) {
      provisioningError.value = e instanceof Error ? e.message : String(e);
      // 失败时保留用户可能已填写的值，不覆盖
    } finally {
      provisioningLoading.value = false;
    }
  },
);

// ESC 关闭
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && !connecting.value) emit('close');
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      window.addEventListener('keydown', onKey);
    } else {
      window.removeEventListener('keydown', onKey);
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey);
});

function setField(key: keyof typeof form.value, v: string) {
  form.value = { ...form.value, [key]: v };
}

// 校验 server 字段：须带 http:// 或 https:// scheme + hostname / IPv4 / [IPv6] + :port
// 返回 null 表示合法，否则返回中文错误描述
function validateServer(input: string): string | null {
  const s = input.trim();
  if (!s) return '服务器地址不能为空';

  // scheme
  let rest: string;
  if (s.startsWith('https://')) {
    rest = s.slice(8);
  } else if (s.startsWith('http://')) {
    rest = s.slice(7);
  } else {
    return '地址须以 http:// 或 https:// 开头';
  }
  if (!rest) return 'scheme 后须接 host:port';

  let host: string;
  let portStr: string;
  if (rest.startsWith('[')) {
    // IPv6 字面量：[::1]:8080
    const close = rest.indexOf(']');
    if (close < 1) return 'IPv6 地址缺少 ]';
    host = rest.slice(1, close);
    if (rest[close + 1] !== ':') return 'IPv6 地址后必须跟 :port';
    portStr = rest.slice(close + 2);
  } else {
    const colon = rest.lastIndexOf(':');
    if (colon <= 0) return '格式应为 scheme://host:port';
    host = rest.slice(0, colon);
    portStr = rest.slice(colon + 1);
  }

  if (!host) return '主机名不能为空';
  if (host.length > 253) return '主机名过长（>253）';
  // 主机名合法字符：字母数字 . - （IPv4 也满足；IPv6 已单独处理）
  if (!/^[A-Za-z0-9.\-]+$/.test(host)) return '主机名含非法字符';
  if (!/^\d{1,5}$/.test(portStr)) return '端口必须为 1-5 位数字';
  const port = parseInt(portStr, 10);
  if (port < 1 || port > 65535) return '端口范围 1-65535';
  return null;
}

async function handleSubmit() {
  if (!form.value.ssid || !form.value.server || !form.value.token) {
    resultOk.value = false;
    resultMsg.value = '请完整填写 SSID / 服务器地址 / token';
    return;
  }
  const serverErr = validateServer(form.value.server);
  if (serverErr) {
    resultOk.value = false;
    resultMsg.value = `服务器地址无效：${serverErr}`;
    return;
  }
  resultMsg.value = null;
  const result = await configure(form.value);
  resultOk.value = result.ok;
  resultMsg.value = result.message;
}

function onBackdropClick(e: MouseEvent) {
  if (e.target === e.currentTarget && !connecting.value) emit('close');
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm"
    @click="onBackdropClick"
  >
    <div class="relative w-full max-w-lg mx-4 bg-surface border border-border-bright shadow-2xl">
      <!-- 顶部条 -->
      <div class="flex items-center justify-between border-b border-border px-4 py-3">
        <div class="flex items-center gap-2">
          <span class="w-1 h-4 bg-accent" />
          <h2 class="font-mono text-xs uppercase tracking-[0.3em] text-ink">
            web serial · provisioning
          </h2>
        </div>
        <button
          type="button"
          @click="emit('close')"
          :disabled="connecting"
          class="text-ink-faint hover:text-ink disabled:opacity-30 transition-colors"
          aria-label="关闭"
        >
          ✕
        </button>
      </div>

      <!-- 兼容性提示 -->
      <div
        v-if="!supported"
        class="px-4 py-2 bg-danger/10 border-b border-danger/30 font-mono text-[10px] text-danger"
      >
        ⚠ 当前浏览器不支持 Web Serial API，请使用 Chrome/Edge 89+
      </div>

      <form @submit.prevent="handleSubmit" class="p-4 space-y-3">
        <!-- WiFi SSID -->
        <label class="block">
          <div class="font-mono text-[9px] uppercase tracking-[0.2em] text-ink-faint mb-1">
            WiFi SSID
          </div>
          <input
            type="text"
            :value="form.ssid"
            @input="(e) => setField('ssid', (e.target as HTMLInputElement).value)"
            placeholder="your-network-name"
            :disabled="connecting"
            autofocus
            class="w-full bg-surface-2 border border-border-bright focus:border-accent focus:outline-none px-3 py-2 text-sm text-ink placeholder:text-ink-faint/50 disabled:opacity-50 transition-colors font-sans"
            autocomplete="off"
            :spellcheck="false"
          />
        </label>

        <!-- WiFi 密码 -->
        <label class="block">
          <div class="font-mono text-[9px] uppercase tracking-[0.2em] text-ink-faint mb-1">
            WiFi 密码
          </div>
          <input
            type="password"
            :value="form.password"
            @input="(e) => setField('password', (e.target as HTMLInputElement).value)"
            placeholder="••••••••"
            :disabled="connecting"
            class="w-full bg-surface-2 border border-border-bright focus:border-accent focus:outline-none px-3 py-2 text-sm text-ink placeholder:text-ink-faint/50 disabled:opacity-50 transition-colors font-sans"
            autocomplete="off"
            :spellcheck="false"
          />
        </label>

        <!-- 服务器地址 -->
        <label class="block">
          <div class="font-mono text-[9px] uppercase tracking-[0.2em] text-ink-faint mb-1">
            服务器地址 (scheme://host:port)
          </div>
          <input
            type="text"
            :value="form.server"
            @input="(e) => setField('server', (e.target as HTMLInputElement).value)"
            placeholder="http://192.168.1.10:8080"
            :disabled="connecting || provisioningLoading"
            class="w-full bg-surface-2 border border-border-bright focus:border-accent focus:outline-none px-3 py-2 text-sm text-ink placeholder:text-ink-faint/50 disabled:opacity-50 transition-colors font-sans"
            autocomplete="off"
            :spellcheck="false"
          />
        </label>

        <!-- 设备 token -->
        <label class="block">
          <div class="font-mono text-[9px] uppercase tracking-[0.2em] text-ink-faint mb-1">
            设备 token
          </div>
          <input
            type="text"
            :value="form.token"
            @input="(e) => setField('token', (e.target as HTMLInputElement).value)"
            placeholder="Bearer ..."
            :disabled="connecting || provisioningLoading"
            class="w-full bg-surface-2 border border-border-bright focus:border-accent focus:outline-none px-3 py-2 text-sm text-ink placeholder:text-ink-faint/50 disabled:opacity-50 transition-colors font-mono"
            autocomplete="off"
            :spellcheck="false"
          />
        </label>

        <!-- 自动填充状态 -->
        <div
          v-if="provisioningLoading"
          class="px-3 py-2 border-l-2 border-accent bg-accent/10 text-accent font-mono text-[11px]"
        >
          ⟳ 正在读取后端配置…
        </div>
        <div
          v-else-if="provisioningError"
          class="px-3 py-2 border-l-2 border-warning bg-warning/10 text-warning font-mono text-[11px]"
        >
          读取配置失败：{{ provisioningError }}，请手动填写 server / token。
        </div>

        <!-- 结果提示 -->
        <div
          v-if="resultMsg"
          :class="[
            'px-3 py-2 border-l-2 font-mono text-[11px]',
            resultOk
              ? 'bg-accent/10 border-accent text-accent'
              : 'bg-danger/10 border-danger text-danger',
          ]"
        >
          {{ resultMsg }}
        </div>

        <!-- 日志区 -->
        <div
          v-if="log.length > 0"
          class="bg-black/60 border border-border p-2 max-h-32 overflow-y-auto font-mono text-[10px] text-ink-dim space-y-0.5"
        >
          <div v-for="(line, i) in log" :key="i" class="leading-tight">{{ line }}</div>
        </div>

        <div class="flex items-center justify-between pt-2">
          <div class="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
            {{ connecting ? 'working…' : 'ready' }}
          </div>
          <div class="flex items-center gap-2">
            <button
              type="button"
              @click="emit('close')"
              :disabled="connecting"
              class="px-3 py-2 font-mono text-[10px] uppercase tracking-wider text-ink-dim hover:text-ink border border-border-bright hover:border-ink-faint disabled:opacity-30 transition-colors"
            >
              cancel
            </button>
            <button
              type="submit"
              :disabled="connecting || !supported"
              class="px-4 py-2 font-mono text-[10px] uppercase tracking-wider bg-accent text-black hover:bg-accent-dim disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            >
              {{ connecting ? '⟳ provisioning…' : '⚡ connect & send' }}
            </button>
          </div>
        </div>
      </form>
    </div>
  </div>
</template>
