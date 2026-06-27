# Fnk0085 智能小车

基于 ESP32-S3（带摄像头）的智能小车系统：Web 端选择设备后即可 WASD 远程控制、无极调速、实时视频流、拍照、Web Serial 配网。

S3 承担摄像头采集、电机控制、编码器测速、PID 平衡等全部车载功能；Rust 后端做设备多路转发与 1s 视频缓存；前端 PWA 提供低延时控制台。

## 架构

```
                       ┌─────────────┐  multipart/x-mixed-replace  ┌─────────────┐
                       │  Web PWA    │ ◀──────────────────────────│  nginx 反代 │
                       │ Vite+Tailwind│                            │  TLS+HTTP/3 │
                       │ Web Worker  │ ─────控制指令─────────────▶│  HTTPS/443  │
                       └─────────────┘                            └──────┬──────┘
                                                                          │ 明文 HTTP
                                                                          │ (h2c, 8080)
                            ┌──────────────┐  HTTP/HTTPS POST frame    ┌─▼──────────┐
                            │  Rust 后端   │ ◀────────────────────────│  ESP32-S3  │
                            │ actix-http   │  HTTP/HTTPS 长轮询 ──────│ 摄像头/电机 │
                            │ 多设备转发    │ ───────────────────────▶│ 编码器/PID  │
                            │ per-device   │   register/poll/event    └────────────┘
                            └──────────────┘
```

后端走明文 HTTP 单端口（默认 8080），TLS + HTTP/3 由 nginx 反代统一处理。设备端根据 `server` 配置字段 scheme 选择 HTTP（直连后端）或 HTTPS（经 nginx）：
- **视频平面**：POST `/api/device/{id}/frame`（body 原始 JPEG，10fps、≤100ms 延时），HTTP/1.1 keep-alive 复用客户端；前端通过 `GET /api/stream/{id}` 的 `multipart/x-mixed-replace` 消费
- **控制平面**：长轮询拉指令、POST 上报事件；固件按 `server` scheme 选择 `WiFiClient`（明文）或 `WiFiClientSecure::setInsecure()`（信任所有证书）；DNS 显式设为 `119.29.29.29` + `8.8.8.8`，SNTP 仅作日志辅助（setInsecure 跳过证书时间校验）

- **固件** ([firmware/](firmware/))：Arduino `.ino`，QVGA 10fps（`jpeg_quality=5`、`fb_count=2` 双缓冲）、L298N 双电机、双槽型光电编码器、PID 平衡 + PWM 缓存、UXGA 拍照存 SD（SD_MMC setPins 39/38/40）、Web Serial 配网 + token 串口打印、HTTP/HTTPS 双模式视频帧上传（`httpsPostFrame` + FreeRTOS Mutex 保护共享客户端）、HTTP/HTTPS 双模式控制通道（`setInsecure` + DNS 119.29.29.29 + FreeRTOS `pollTask`）
- **后端** ([backend/](backend/))：Rust + actix-http v3.13.1（启 http2 + compress-gzip feature，明文 h2c 协商），多设备 DashMap、每设备 1s 视频环形缓存 + per-device 指令队列、gzip、token 校验、`include_dir` 内嵌前端单二进制部署
- **前端** ([frontend/](frontend/))：Vite 8.1.0 + Vue 3.5.9 + TailwindCSS 4.3.1，工业控制台美学、WASD + 1-100 调速滑块、Web Worker 取流 + 延时显示、拍照加载圈、Web Serial 配网（含 host:port 与 scheme 校验）、PWA injectManifest 离线缓存
- **协议** ([protocol.md](protocol.md))：HTTP/HTTPS 视频帧上传 + HTTP/HTTPS 设备 API + 浏览器 API + Web Serial 帧格式

## 功能特性

| 模块 | 能力 |
|------|------|
| 远程控制 | WASD 四向 + 1-100 无极调速（软件映射 0-255 PWM） |
| 视频流 | QVGA 10fps（quality=5），HTTPS POST 单帧 + multipart/x-mixed-replace 下发，目标 ≤100ms 延时 |
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

首次启动在运行目录生成 `Fnk0085-smart-car-config.jsonc`（含端口/token/缓存时长/日志级别，支持注释）。后端走明文 HTTP（默认 8080，启 `http2` feature 支持 h2c 协商，HTTP/1.1 回退）。修改配置后重启。

> Windows 路径含中文可能导致 build script panic，可设 `$env:CARGO_TARGET_DIR = "C:\temp\fnk0085-target"` 绕过。

### 2. nginx 反代（生产部署，可选）

后端明文 HTTP 仅监听内网端口；对外由 nginx 终止 TLS（HTTP/2 over TLS + HTTP/3 over QUIC）并反代到后端 8080：

```nginx
server {
    listen 443 ssl;
    listen 443 quic reuseport;
    http2 on;
    http3 on;
    server_name car.example.com;

    ssl_certificate     /etc/nginx/certs/fullchain.pem;
    ssl_certificate_key /etc/nginx/certs/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header Connection "";
        proxy_buffering off;            # multipart/x-mixed-replace 视频流禁缓冲
        proxy_read_timeout 3600s;      # 长轮询 / 视频流
    }
}
```

固件 `server` 字段填 `https://car.example.com`（经 nginx）或 `http://<内网IP>:8080`（直连后端）均可。

### 3. 前端（独立开发预览）

```powershell
cd frontend
bun install
bun run build      # 产物到 dist/，后端会内嵌
bun run dev        # 开发模式
```

### 4. 固件

1. 用 Arduino IDE 打开 [firmware/Fnk0085-smart-car/Fnk0085-smart-car.ino](firmware/Fnk0085-smart-car/Fnk0085-smart-car.ino)
2. 安装依赖：`ESP32 Arduino` 核心 3.x、`ArduinoJson`；SD_MMC / WiFiClientSecure / WiFiClient / HTTPClient / time 已随核心自带
3. 选板子 `ESP32S3 Dev Module`，启用 PSRAM，烧录
4. 烧录后串口（115200）会进入配网等待状态，发送 `CONFIG\n` 可查询已存配置
5. 通过前端 Web Serial 弹窗一键下发 `CONFIG|ssid=...|password=...|server=<scheme>://<host:port>|token=...` 至 NVS（`server` 字段需带 `http://` 或 `https://` scheme）；设备收到后自动重启并连接
6. 参考引脚见 [camera_pins.h](firmware/Fnk0085-smart-car/camera_pins.h)；SD_MMC 引脚硬连线 CLK=39 / CMD=38 / D0=40（与 Freenove 板一致）

### 5. 访问

- 生产：浏览器打开 `https://<域名>/`（nginx 提供 HTTPS + HTTP/3 + 合法证书）
- 开发：浏览器打开 `http://<服务器>:8080/`（直连后端明文 HTTP）

选择设备即可控制。

## 技术栈

- **固件**：ESP32 Arduino、WiFiClientSecure（`setInsecure()` 跳过证书校验）+ WiFiClient（明文 HTTP）、HTTPClient（HTTP/1.1 keep-alive）、LEDC PWM、Camera Driver、FreeRTOS Mutex、`WiFi.config()` DNS 119.29.29.29 + 8.8.8.8
- **后端**：Rust stable、actix-http 3.13.1（`http2` + `compress-gzip` feature，明文 h2c）、DashMap、tokio broadcast、include_dir、build.rs 自动构建前端
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
