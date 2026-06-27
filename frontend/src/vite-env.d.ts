/// <reference types="vite/client" />

// 编译期注入的应用版本号（来自 package.json），用于 PWA 缓存键
declare const __APP_VERSION__: string;

// vite-plugin-pwa 注入的虚拟模块
declare module 'virtual:pwa-register' {
  export interface RegisterSWOptions {
    immediate?: boolean;
    onNeedRefresh?: () => void;
    onOfflineReady?: () => void;
    onRegistered?: (
      registration: ServiceWorkerRegistration | undefined,
    ) => void;
    onRegisterError?: (error: unknown) => void;
  }
  export function registerSW(
    options?: RegisterSWOptions,
  ): (reloadPage?: boolean) => Promise<void>;
}
