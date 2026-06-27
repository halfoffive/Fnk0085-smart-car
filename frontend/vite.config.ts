import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import { VitePWA } from 'vite-plugin-pwa';

// PWA 缓存版本：与项目版本号一致，构建期注入到 SW 与缓存键
const APP_VERSION = JSON.parse(
  readFileSync(fileURLToPath(new URL('./package.json', import.meta.url)), 'utf-8'),
).version as string;

// https://vitejs.dev/config/
export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(APP_VERSION),
  },
  plugins: [
    vue(),
    VitePWA({
      // 使用 injectManifest 模式：自定义 SW 直接 ESM 导入 workbox，
      // 避免 generateSW 模式注入的 module shim 异步加载导致 precache install handler 注册晚于 SW install 事件
      strategies: 'injectManifest',
      registerType: 'autoUpdate',
      injectRegister: 'auto',
      srcDir: 'src',
      filename: 'sw.ts',
      // 不让 vite-plugin-pwa 自动生成 manifest（其 0.20.5 版本在 Vite 8 Rolldown 下生成失败）
      // 改为 public/manifest.webmanifest 静态提供，vite 构建时复制到 dist/
      manifest: false,
      injectManifest: {
        globPatterns: ['**/*.{js,css,html,svg,png,woff,woff2,ico,webmanifest}'],
        // 注入 precache 清单的同时把版本号注入到 SW 全局变量
        additionalManifestEntries: undefined,
      },
      devOptions: {
        enabled: false,
      },
    }),
  ],
  build: {
    target: 'es2022',
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks: (id) => {
          if (id.includes('node_modules/vue') || id.includes('node_modules/@vue')) {
            return 'vue';
          }
          return undefined;
        },
      },
    },
  },
  worker: {
    format: 'es',
  },
});
