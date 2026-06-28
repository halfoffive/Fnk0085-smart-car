/// <reference lib="webworker" />
// Fnk0085 PWA Service Worker
// 直接 ESM 导入 workbox，避免 generateSW 模式的 module shim 异步加载导致 precache install handler 注册晚于 SW install 事件
import { precacheAndRoute, cleanupOutdatedCaches } from 'workbox-precaching';
import { clientsClaim, setCacheNameDetails } from 'workbox-core';
import { registerRoute } from 'workbox-routing';
import { StaleWhileRevalidate, NetworkFirst, NetworkOnly } from 'workbox-strategies';
import { ExpirationPlugin } from 'workbox-expiration';
import { CacheableResponsePlugin } from 'workbox-cacheable-response';

// vite-plugin-pwa injectManifest 模式会在构建时把 precache 清单注入到 self.__WB_MANIFEST
declare const self: ServiceWorkerGlobalScope & {
  __WB_MANIFEST: Array<{ url: string; revision: string | null }>;
};

// 版本号由 vite define 注入（__APP_VERSION__），用于缓存键与日志
declare const __APP_VERSION__: string;

const APP_VERSION: string = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : 'dev';
const CACHE_PREFIX = `fnk0085-v${APP_VERSION}`;

// 统一缓存命名：所有 workbox 缓存都带 fnk0085-v<version> 前缀，
// 便于版本更新时通过 caches.keys() 过滤清理
setCacheNameDetails({
  prefix: CACHE_PREFIX,
  suffix: 'v2',
  precache: 'precache',
  runtime: 'runtime',
});

// precache 所有构建产物（globPatterns 在 vite.config.ts 中配置）
precacheAndRoute(self.__WB_MANIFEST);

// 清理旧版本缓存（版本号变化时自动清理）
cleanupOutdatedCaches();
clientsClaim();

// 跳过等待，新 SW 立即生效
self.addEventListener('message', (event) => {
  if (event.data === 'SKIP_WAITING') {
    self.skipWaiting();
  }
});

// 字体走 Google Fonts CDN：StaleWhileRevalidate
registerRoute(
  ({ url }) =>
    url.origin === 'https://fonts.googleapis.com' ||
    url.origin === 'https://fonts.gstatic.com',
  new StaleWhileRevalidate({
    cacheName: `${CACHE_PREFIX}-fonts`,
    plugins: [
      new CacheableResponsePlugin({ statuses: [0, 200] }),
      new ExpirationPlugin({
        maxEntries: 20,
        maxAgeSeconds: 60 * 60 * 24 * 365, // 1 年
      }),
    ],
  }),
);

// POST 请求直接走网络，不缓存响应（控制/拍照等接口）
registerRoute(
  ({ url, request }) =>
    url.pathname.startsWith('/api/') && request.method === 'POST',
  new NetworkOnly()
);

// API（非流式 GET）走 network-first，3s 超时回退缓存
registerRoute(
  ({ url, request }) =>
    /\/api\/(devices|pwm_cache|control|photo)/.test(url.pathname) &&
    request.method === 'GET',
  new NetworkFirst({
    cacheName: `${CACHE_PREFIX}-api`,
    networkTimeoutSeconds: 3,
    plugins: [
      new CacheableResponsePlugin({ statuses: [0, 200] }),
      new ExpirationPlugin({
        maxEntries: 50,
        maxAgeSeconds: 60 * 5, // 5 分钟
      }),
    ],
  }),
);

// 视频流 /api/stream/* 不缓存（流式响应），直接走网络
// （registerRoute 默认不匹配即不拦截，视频流由 fetch 直接处理）

self.addEventListener('install', () => {
  // precacheAndRoute 已自动注册 install handler，这里仅日志
  console.log(`[Fnk0085 SW] install v${APP_VERSION}`);
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      // 清理非当前版本的缓存
      const keys = await caches.keys();
      await Promise.all(
        keys
          .filter((k) => k.startsWith('fnk0085-v') && !k.startsWith(CACHE_PREFIX))
          .map((k) => caches.delete(k)),
      );
      console.log(`[Fnk0085 SW] activate v${APP_VERSION}, cleaned ${keys.length} old caches`);
    })(),
  );
});
