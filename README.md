# Fnk0085 智能小车

基于 ESP32-S3（带摄像头）的智能小车系统：Web 端选择设备后即可 WASD 远程控制、无极调速、实时视频流、拍照、Web Serial 配网。

S3 承担摄像头采集、电机控制、编码器测速、PID 平衡等全部车载功能；Rust 后端做设备多路转发与 1s 视频缓存；前端 PWA 提供低延时控制台。

## 架构

```
┌─────────────┐   UDP+AEAD(8分包)   ┌──────────────┐   HTTPS+SSE   ┌─────────────┐
│  ESP32-S3   │ ──────────────────▶ │  Rust 后端   │ ────────────▶│  Web PWA    │
│ 摄像头/电机 │ ◀────────────────── │ actix-http   │ ◀────────────│ Vite+Tailwind│
│ 编码器/PID  │   UDP+AEAD(指令)    │ 多设备转发    │   控制指令     │ Web Worker  │
└─────────────┘                     └──────────────┘              └─────────────┘
```

- **固件** ([firmware/](firmware/))：Arduino `.ino`，QVGA 10fps、L298N 双电机、双槽型光电编码器、PID 平衡 + PWM 缓存、UXGA 拍照存 SD、Web Serial 配网、AES-128-GCM AEAD（等价 DTLS）
- **后端** ([backend/](backend/))：Rust + actix-http v3.13.1，多设备 DashMap、每设备 1s 视频环形缓存、gzip、token+SSL（mTLS）、`include_dir` 内嵌前端单二进制部署
- **前端** ([frontend/](frontend/))：Vite 8.1.0 + Vue 3.5.9 + TailwindCSS 4.3.1，工业控制台美学、WASD + 1-100 调速滑块、Web Worker 取流 + 延时显示、拍照加载圈、Web Serial 配网、PWA injectManifest 离线缓存
- **协议** ([protocol.md](protocol.md))：UDP 分包字节布局 + HTTP API + Web Serial 帧格式

## 功能特性

| 模块 | 能力 |
|------|------|
| 远程控制 | WASD 四向 + 1-100 无极调速（软件映射 0-255 PWM） |
| 视频流 | QVGA 10fps，UDP 8 分包 + AEAD，目标 ≤100ms 延时 |
| 拍照 | 暂停视频流 → UXGA 最高清晰度 → 存 SD 卡 → 恢复流 |
| 智能修正 | PID 平衡双轮转速，稳定后缓存 PWM，下次直接输出（可关闭） |
| 配网 | Web Serial API 一键下发 WiFi/服务器/token 至 ESP32 NVS |
| 多设备 | 后端同时转发多设备，前端下拉切换 |
| 离线 | PWA Service Worker 缓存全部前端文件，断网可访问 |

## 快速开始

### 1. 后端（含前端自动构建）

```powershell
cd backend
cargo run --release
```

`backend/build.rs` 会在编译期自动检测 `frontend/` 状态：`node_modules` 缺失则 `bun install`，`dist/` 缺失或源码较新则 `bun run build`，再 `include_dir!` 内嵌。日常只需 `cargo run` 即可。

首次启动在运行目录生成 `Fnk0085-smart-car-config.jsonc`（含端口/token/TLS 路径，支持注释）与 `certs/`（rcgen 自签）。修改配置后重启。

> Windows 路径含中文可能导致 build script panic，可设 `$env:CARGO_TARGET_DIR = "C:\temp\fnk0085-target"` 绕过。

### 2. 前端（独立开发预览）

```powershell
cd frontend
bun install
bun run build      # 产物到 dist/，后端会内嵌
bun run dev        # 开发模式
```

### 3. 固件

1. 用 Arduino IDE 打开 [firmware/Fnk0085-smart-car/Fnk0085-smart-car.ino](firmware/Fnk0085-smart-car/Fnk0085-smart-car.ino)
2. 安装依赖：`ESP32 Arduino` 核心、`ArduinoJson`、`AESLib`（或等价 AEAD 库）
3. 修改顶部配置：WiFi SSID/密码、后端地址、token
4. 选板子 `ESP32S3 Dev Module`，启用 PSRAM，烧录
5. 参考引脚见 [camera_pins.h](firmware/Fnk0085-smart-car/camera_pins.h)

### 4. 访问

浏览器打开 `https://<服务器>:8080/`（自签证书需手动信任），选择设备即可控制。

## 技术栈

- **固件**：ESP32 Arduino、AES-128-GCM AEAD、LEDC PWM、Camera Driver
- **后端**：Rust stable、actix-http 3.13.1、rustls 0.23、DashMap、tokio broadcast、include_dir、build.rs 自动构建前端
- **前端**：Vite 8.1.0、Vue 3.5.9、TailwindCSS 4.3.1、Web Worker、Web Serial API、vite-plugin-pwa（injectManifest）、bun

## 开发板参考

[Freenove ESP32-S3 WROOM](https://docs.freenove.com/en/latest/index.html)（含摄像头 + IO 扩展）

## 目录结构

```
Fnk0085-smart-car/
├── firmware/      # ESP32-S3 .ino
├── backend/       # Rust actix-http
├── frontend/      # Vite PWA
├── protocol.md    # 通信协议
├── README.md
└── CHANGELOG.md
```

## 协议

详见 [protocol.md](protocol.md)。
