<script setup lang="ts">
// 访问密码登录页：未登录时全屏展示，登录成功后 emit('authed')
import { ref } from 'vue';
import { postLogin } from '../lib/api';
import { setAuthed } from '../lib/auth';

const password = ref('');
const error = ref('');
const loading = ref(false);

const emit = defineEmits<{ authed: [] }>();

async function submit() {
  loading.value = true;
  error.value = '';
  const ok = await postLogin(password.value);
  loading.value = false;
  if (ok) {
    setAuthed();
    emit('authed');
  } else {
    error.value = '密码错误';
  }
}
</script>

<template>
  <div class="h-screen w-screen flex items-center justify-center bg-base text-ink overflow-hidden">
    <div class="w-full max-w-sm mx-4 bg-surface border border-border">
      <!-- 顶部条 -->
      <div class="flex items-center justify-between border-b border-border px-4 py-3">
        <div class="flex items-center gap-2">
          <span class="w-1 h-4 bg-accent" />
          <h2 class="font-mono text-xs uppercase tracking-[0.3em] text-ink">
            Fnk<span class="text-accent">0085</span>
          </h2>
        </div>
        <span class="font-mono text-[9px] uppercase tracking-[0.25em] text-ink-faint">
          smart car console
        </span>
      </div>

      <form @submit.prevent="submit" class="p-4 space-y-3">
        <!-- 访问密码 -->
        <label class="block">
          <div class="font-mono text-[9px] uppercase tracking-[0.2em] text-ink-faint mb-1">
            access password
          </div>
          <input
            type="password"
            v-model="password"
            placeholder="••••••••"
            autofocus
            autocomplete="off"
            :spellcheck="false"
            @keyup.enter="submit"
            class="w-full bg-surface-2 border border-border-bright focus:border-accent focus:outline-none px-3 py-2 text-sm text-ink placeholder:text-ink-faint/50 transition-colors font-mono"
          />
        </label>

        <!-- 错误提示 -->
        <div
          v-if="error"
          class="px-3 py-2 border-l-2 border-danger bg-danger/10 text-danger font-mono text-[11px]"
        >
          ✕ {{ error }}
        </div>

        <div class="flex items-center justify-between pt-2">
          <div class="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
            {{ loading ? 'verifying…' : 'ready' }}
          </div>
          <button
            type="submit"
            :disabled="loading || !password"
            class="px-4 py-2 font-mono text-[10px] uppercase tracking-wider bg-accent text-black hover:bg-accent-dim disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            {{ loading ? '...' : '⚡ unlock' }}
          </button>
        </div>
      </form>
    </div>
  </div>
</template>
