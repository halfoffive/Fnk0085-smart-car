// 全局常量：API 基址、版本号、轮询间隔等

// 应用版本（编译期由 vite.config.ts define 注入，对应 package.json version）
export const APP_VERSION: string =
  typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.0.0-dev';

// API 基址：开发态走 Vite proxy / 同源；生产态后端会内嵌前端 dist，所以同源即可
const DEV_API_BASE = 'http://localhost:8080';
const PROD_API_BASE = '';

export const API_BASE: string = import.meta.env.DEV
  ? (import.meta.env.VITE_API_BASE ?? DEV_API_BASE)
  : PROD_API_BASE;

// 设备列表轮询间隔
export const DEVICE_POLL_INTERVAL_MS = 2000;

// 调速滑块 debounce 间隔
export const PWM_DEBOUNCE_MS = 200;

// PWM 取值范围（设备端 LEDC 8 位分辨率）
export const PWM_MIN = 0;
export const PWM_MAX = 255;

// 滑块 UI 范围
export const SLIDER_MIN = 1;
export const SLIDER_MAX = 100;

// Web Serial 串口波特率
export const SERIAL_BAUD_RATE = 115200;

// 串口配网响应超时
export const SERIAL_RESPONSE_TIMEOUT_MS = 10000;

// 视频流默认刷新帧率（仅用于骨架动画）
export const STREAM_FPS_TARGET = 10;
