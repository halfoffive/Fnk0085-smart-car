# Changelog

本项目版本号统一：固件 / 后端 / 前端共享 `0.3.0`。前端 PWA 缓存键带版本号，版本变更时自动清理旧缓存。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.3.0] - 2026-06-27

### BREAKING
- 后端移除 rustls 0.23 / rcgen / rustls-pemfile / tokio-rustls 依赖与 `rustls-0_23` feature；改为明文 HTTP（保留 `http2` feature 支持 h2c 协商），TLS + HTTP/3 由 nginx 反代统一处理
- 后端移除 `GET /api/ca_cert` 端点与 `handlers/ca_cert.rs`；`AuthConfig` 移除 `tls_cert` / `tls_key` / `client_ca` / `ca_cert` 字段；`AppState` 移除 `ca_cert_path` 字段
- 固件删除 `firmware/Fnk0085-smart-car/ca_cert.h`；`setCACert(root_ca_pem)` 改回 `setInsecure()` 信任所有证书（开发期方案，生产由 nginx 提供合法证书）

### Added
- 固件 HTTP/HTTPS 双模式：根据 `server` 配置字段 scheme（`http://` 或 `https://`）选择 `WiFiClient`（明文）或 `WiFiClientSecure`（setInsecure），`buildUrl` 与 `httpsPost` / `httpsGet` / `httpsPostFrame` 全部 scheme-aware
- 固件 DNS 显式配置：`WiFi.config()` 在 `WiFi.begin()` 前设置 DNS1=`119.29.29.29`（DNSPod）+ DNS2=`8.8.8.8`（Google），DHCP 自动获取 IP/Gateway/Subnet
- 固件 `stripServerScheme()` helper：解析 `server` 字段的 `http://` / `https://` 前缀，剥离后存 `backendHost` + `backendPort`，兼容无 scheme 老配置（默认走 HTTPS）
- 后端 `HttpService::build().finish(handler).tcp_auto_h2c()` 明文 HTTP/2（h2c）协商，回退 HTTP/1.1

### Changed
- `protocol.md` §1 / §2 / §5 更新为「后端明文 HTTP + nginx TLS 终止 + 固件 setInsecure 双模式」；移除 §3.7 CA 端点章节；附录协议版本号 2→3
- `AGENTS.md` 仓库结构、配置与密钥、协议改动清单更新：移除 CA 烧写流程，新增 nginx 反代 + DNS 119.29.29.29 说明
- `README.md` 架构图与部署流程更新：移除 CA 抓取步骤，新增 nginx 反代部署说明

### Removed
- 后端 `backend/src/handlers/ca_cert.rs` 整文件删除
- 后端 `Cargo.toml` 移除 `rustls` / `rustls-pemfile` / `tokio-rustls` / `rcgen` / `time` 依赖
- 后端 `config.rs` 移除 `gen_self_signed_pair` / `load_or_generate_tls_materials` / `TlsMaterials` 类型与相关测试
- 固件 `firmware/Fnk0085-smart-car/ca_cert.h` 文件删除（含 PROGMEM `root_ca_pem[]` 常量）

### Security
- 设备侧 HTTPS 客户端改回 `WiFiClientSecure::setInsecure()` 信任所有证书（开发期方案；生产由 nginx 提供合法证书，固件无需感知 CA 轮换）
- 后端明文 HTTP 仅监听内网端口，对外由 nginx 终止 TLS（HTTP/2 over TLS + HTTP/3 over QUIC），固件经 nginx 转发或直连后端均可

## [0.2.0] - 2026-06-27

### BREAKING
- 弃用 UDP+AEAD 视频通道，全面转向 HTTPS：视频帧走 `POST /api/device/{id}/frame`，body 原始 JPEG
- 设备端 TLS 从 `setInsecure()` 改为 `setCACert()` CA 固定，需烧写前将后端 `certs/ca.crt` 拷贝至 `firmware/Fnk0085-smart-car/ca_cert.h`

### Added
- 后端新增 `POST /api/device/{id}/frame` 端点接收设备视频帧
- 后端新增 `GET /api/ca_cert` 端点暴露 CA PEM
- 固件新增 SNTP 时间同步（`configTime` + 5s 超时 + 证书宽限期 1970-2099 兜底）
- 固件新增 `httpsPostFrame` 与 FreeRTOS Mutex 互斥保护共享 `httpsClient`
- 固件 `ca_cert.h` 编译期内嵌 CA PEM

### Changed
- 固件摄像头 `jpeg_quality` 10→5（更高图像质量）、`fb_count` 1→2（双缓冲）
- 后端证书有效期改为 1970-01-01 至 2099-12-31（兜底 SNTP 失败场景）
- `protocol.md` §1 重写为 HTTPS 视频帧上传；§3.4 描述对齐 multipart/x-mixed-replace 实现；§5 鉴权改为 CA pinning + Bearer token

### Removed
- 后端 `udp_listener.rs` 与 `crypto.rs` 整文件删除
- 后端 `AppState.key`（AeadKey）字段删除；`Cargo.toml` 移除 `aes-gcm`/`sha2` 依赖
- 固件 UDP 发送栈（`udpSend`、`aesKey`、`deriveAesKey`、`aeadEncrypt`、`sendVideoFrame`）、`BACKEND_UDP_PORT` 常量、mbedtls AEAD 头文件
- `protocol.md` 原 §1 UDP 分包章节

## [Unreleased]

### Changed
- 设备控制通道由 UDP 迁移至 HTTPS 长轮询（视频流仍走 UDP+AEAD）：
  - 后端新增 `/api/device/{id}/register`、`/api/device/{id}/poll?timeout=N`、`/api/device/{id}/event` 三个端点
  - 每设备 `mpsc::channel(16)` 指令队列 + `AsyncMutex<Receiver>`，poll 长轮询消费
  - UDP 监听任务仅保留视频流（移除 `outbound_loop` / `handle_event` / `validate_token` / `find_device_by_addr`）
  - `AppState` 移除 `cmd_sink: CommandSink`，全局指令通道改为 per-device
- 固件控制平面迁移至 HTTPS（`WiFiClientSecure::setInsecure()` 信任自签证书）：
  - `sendRegister` / `sendPhotoDone` / `sendAck` / `sendError` 改为 `POST /api/device/{id}/register` 与 `POST /api/device/{id}/event`
  - 新增 `pollTask`（FreeRTOS，core 0）长轮询 `GET /api/device/{id}/poll?timeout=30`，指令通过 `xQueueSend` 投递给主 `loop`
  - 主 `loop` 由 `pollUdpRecv` 改为 `dispatchCommands`（从 `cmdQueue` 拉取并派发到 handler）
  - 移除 `udpRecv` 套接字（UDP 仅保留视频流 `udpSend`）
- 前端框架由 React 18 迁移至 Vue 3.5.9（SFC + `<script setup lang="ts">` + composables），包管理器由 npm 切换为 **bun**
- 后端新增 `backend/build.rs`：编译期自动检测 `frontend/node_modules` 与 `frontend/dist`，按需执行 `bun install` / `bun run build`，再 `include_dir!` 内嵌；日常 `cargo build` 即可

### Added
- 固件 SD 卡初始化修复：`SD_MMC.setPins(39, 38, 40)` + 5 参数 `begin("/sdcard", true, true, SDMMC_FREQ_DEFAULT, 5)`，与 Freenove 示例 Sketch_07.3 一致；启动时打印卡类型 / 容量 / 总空间
- 固件串口新增 `CONFIG` 查询命令：单行 `CONFIG\n` 打印 NVS 中已存的 ssid / password 长度 / server / token，便于排查配网问题
- 固件 setup / pollSerialConfig 全面增强日志：server 与 UDP 端口分别打印、token 明文打印、端口范围 1-65535 校验、CONFIG 行接收时回显
- 前端 ConfigDialog 服务器地址校验：支持 hostname / IPv4 / `[IPv6]` + `:port`，端口范围 1-65535，非法时阻止提交并提示

### Fixed
- 修复 ESP32-S3 SD 卡挂载失败的问题：原 `SD_MMC.begin("/sdcard", true)` 未调用 `setPins`，ESP32-S3 SD_MMC 走默认引脚导致初始化失败；参考 Freenove 示例补全
- 修复配网成功但后端无日志的问题：原 register 走 UDP，UDP 监听改造后不再处理事件；改为 HTTPS POST `/api/device/{id}/register`，后端正确记录注册日志
- 修复 Chrome 访问 HTTPS 端口报 `ERR_CONNECTION_RESET` 的问题：
  - 根因：actix-http `rustls_0_23` acceptor 强制把 `h2` 注入 ALPN 列表（service.rs:444-446），覆盖我们设置的 `["http/1.1"]`，Chrome 协商到 h2 后 actix-http 因未启用 `http2` feature panic
  - 修复：启用 `actix-http` 的 `http2` feature，同时移除无效的 `alpn_protocols = ["http/1.1"]` 设置（actix-http 默认注入 `[h2, http/1.1]`，浏览器自动协商）
- 修复固件在 ESP32 Arduino 核心 3.3.8-cn 下的编译错误：
  - `ledcSetup` / `ledcAttachPin` 已废弃 → 改用 `ledcAttach(pin, freq, res)` + `ledcWrite(pin, duty)`
  - `ESP.getChipId()` 不再存在 → 改用 `ESP.getEfuseMac()`
  - `mbedtls_sha256_starts_ret` / `_update_ret` / `_finish_ret` 已合并为无后缀版本
  - `volatile ++` 触发 `-Wvolatile` → 改为显式 `x = x + 1`
  - 结构体聚合初始化缺少成员告警 → 显式列出全部字段

### Security
- 设备侧 HTTPS 客户端使用 `WiFiClientSecure::setInsecure()` 信任后端自签证书（开发期方案）；生产部署应换为 mTLS 或 CA pinning（`setCACert()`）

## [0.1.0] - 2026-06-26

### Added
- 初始化 ESP32-S3 固件：QVGA 10fps 摄像头、L298N 双电机驱动、双槽型光电编码器测速、PID 双轮平衡、PWM 缓存表（可关闭）、UXGA 拍照存 SD、Web Serial 配网、设备身份 `ESP32S3_<chipId>_<MAC>`
- 初始化 Rust 后端（actix-http v3.13.1）：多设备 DashMap 注册表、每设备 1s 视频环形缓存、UDP+AEAD 分包重组、gzip、token+SSL（rcgen 自签 + mTLS）、`include_dir` 内嵌前端、`Fnk0085-smart-car-config.jsonc` 配置（JSONC 注释）
- 初始化前端 PWA（Vite 8.1.0 + TailwindCSS 4.3.1）：设备选择、WASD 控制、1-100 无极调速滑块、Web Worker 取流 + 延时显示、拍照加载圈、Web Serial 配网、PWA injectManifest 离线缓存
- 通信协议 [protocol.md](protocol.md)：UDP 8 分包字节布局 + HTTP API + Web Serial 帧格式
- 集成测试 [tests/ui_smoke.py](tests/ui_smoke.py)：Playwright UI 烟雾测试 + PWA 离线验证（27/27 PASS）
- 安全设计：AES-128-GCM AEAD（token 派生密钥 + nonce + 包头作 AAD），等价 DTLS 安全属性

### Security
- 设备 ↔ 后端 UDP 流量经 AES-128-GCM AEAD 加密（Arduino-ESP32 无高层 DTLS API，用 AEAD 等价替代）
- 后端 HTTPS + rustls 0.23，支持客户端证书校验（mTLS）
- token 与 deviceId 绑定，register 阶段校验

### Fixed
- 修复 vite-plugin-pwa generateSW 模式在 Vite 8 Rolldown 下 module shim 异步加载导致 precache install handler 注册晚于 SW install 事件、离线白屏的问题：改用 injectManifest + 自定义 `sw.ts` + `setCacheNameDetails`
- 修复 precache 清单中 `manifest.webmanifest` 重复条目（vite-plugin-pwa 自动生成与 public/ 静态提供双源冲突）：设 `manifest: false`，由 public/ 提供
- 修复 rustls 0.23 API 适配（`with_no_client_auth` / `with_single_cert` 状态机、`HttpService::rustls_0_23`、`ServiceFactoryExt::map_err`）
- 修复后端 16 项 clippy 警告：useless_conversion、manual_flatten、manual_is_multiple_of、needless_borrows_for_generic_args、io_other_error、type_complexity、dead_code 清理
- 修复 UDP 监听 `idle_ticks` 计数逻辑 bug（每次接收被重置导致清理永不触发）

### Known Issues
- 摄像头引脚（GPIO 4/5/6/7/15）与 L298N IN1-4 / 右编码器硬件冲突，需 PCB 调整或 GPIO 扩展
- 后端 `cargo run` 在 Windows 下因 `version_check` crate spawn 子进程失败，建议 Linux/WSL2 复验
- 端到端 ≤100ms 延时与多设备并发需真实 ESP32 硬件联调
