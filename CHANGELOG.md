# Changelog

本项目版本号统一：固件 / 后端 / 前端共享 `0.3.1`。前端 PWA 缓存键带版本号，版本变更时自动清理旧缓存。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.3.1-audit-fix] - 2026-06-28

本次为综合审查后的安全与稳定性修复批次，不升级三端 `package.json` / `Cargo.toml` 版本号，PWA 缓存键保持 `0.3.1`。

### Security

- 前端 Web Serial 配网日志对 `password`/`token` 字段脱敏，避免串口日志泄露敏感信息
- 后端 `/api/auth/login` 使用 `subtle::ConstantTimeEq` 进行恒定时间密码比较，消除时序侧信道
- 固件串口不再打印 token 明文：NVS 查询、`[CFG]` 启动日志、CONFIG 行回显均通过 `***` 掩码

### Fixed

- 固件 `handlePhoto()` 中 `photoMux` 获取超时改为 5000ms，超时路径调用 `sendError(5002, "photo mutex timeout")` 并立即返回，避免 videoTask 异常持锁导致 `loop()` 被阻塞触发看门狗
- 前端 `useKeyboard` 仅在 `App.vue` 挂载一次；释放方向键时若还有其他方向键被按住不再发送 stop；窗口失焦/组件卸载时释放全部方向键
- 后端 `DeviceRegistry::get_or_create` 修复 TOCTOU 竞态：通过 `DashMap::entry` + CAS 名额检查将存在性检查与插入合并为原子操作，并发注册测试不超过 `max_devices`
- 后端 `VideoCache` 增加每设备总字节上限与单帧大小上限，超出时按 LRU 丢弃旧帧，防止高码流下 OOM
- 后端延迟计算从 `SystemTime` 迁移到 `tokio::time::Instant`，`now_ms()` 以进程启动为基准单调递增，避免 NTP 回拨导致 `X-Latency-Ms` 异常
- 后端 `/api/device/{id}/frame` 读取请求头 `X-Device-Uptime-Ms` 并写入 `Frame.device_uptime_ms`，视频流初始响应头与每帧 multipart part 头透传该字段
- 固件 `telemetryTask` 首次 HTTPS POST 失败后回退到明文 `telemetryClient`，与 poll/video 通道行为一致；WiFi 断线重连后调用 `resetNetworkClients()` 停止并重置所有网络客户端，同时复位 `httpsHandshakeFailed`
- 前端 `ControlPanel.vue` 的 `watch` 使用 `onCleanup` 注册清理函数；`VideoStream.vue` 使用响应式 `src` 绑定；`videoWorker.ts` 将 `AbortError` 识别为正常取消，切换设备/停止流时不再报红

### Changed

- `.github/workflows/build-backend.yml` 移除已废弃的 `actions-rs/cargo@v1`；`aarch64-unknown-linux-musl` 改用 pin 到 commit hash 的 `taiki-e/install-action` 安装 `cross` 后执行 `cross build`

## [0.3.1] - 2026-06-27

### Added
- 后端新增 `GET /api/health` 健康端点（无鉴权，返回 `{"status":"ok","version":"0.3.1"}`），用于固件启动 scheme 探测与运维监控
- 固件启动主动探测 HTTP/HTTPS scheme：WiFi 关联 + SNTP 同步后、首次 register 之前调用 `probeScheme()`，先试 NVS 配的 scheme，失败再试另一 scheme，成功则切换并写回 NVS（auto-correct），冷启动不再刷 TLS 错误日志
- 协议新增 `type=telemetry` 上行事件：固件在 `loop()` 100ms 编码器采样处计算左右轮速后，通过 `POST /api/device/{id}/event` 上报 `{"type":"telemetry","leftRpm":N,"rightRpm":N}`；后端 `DeviceEvent` 新增 `Telemetry` variant 并持久化到设备状态；前端 `GET /api/devices` 透传 `leftRpm`/`rightRpm` 并在状态条显示
- 后端新增 `GET /api/telemetry/{deviceId}` 返回设备左右轮速（`{"leftRpm":..., "rightRpm":...}`，设备不在线返回 404）；新增 `GET /api/config` 无鉴权返回 `{"server":"host:port","token":"..."}`，供 Web Serial 配网弹窗自动填充；`AppState` 新增 `log_level`/`server_addr` 字段；路由入口统一输出 `[ACCESS] <method> <path> -> <status> (<duration>ms)` 访问日志，debug 模式额外打印请求头与查询串

### Fixed
- 修复拍照时 SCCB 竞争导致的 `SCCB_Write Failed addr:0x30 ... ret:259` 与 `[PHOTO] capture failed`：引入 `photoMux` 二进制信号量，`handlePhoto()` 独占 SCCB 期间 `videoTask()` 跳过采集；拍照前后增加 300ms/200ms/100ms 延时稳定传感器；拍照失败通过 `sendError(5002, ...)` 立即上报，避免后端 `POST /api/photo/{id}` 504 挂起
- 修复/观测视频流缺失：`httpsPostFrame()` 在非 2xx/204 时打印 `[FRAME] POST failed code=%d len=%u, https=%d`；`videoTask()` 每 10 帧打印 `[VIDEO] frames ok=%u fail=%u`；拍照恢复 QVGA 后视频流继续
- 修复后端 `GET /api/devices` 字段命名（`deviceId`/`lastSeenMs`）与 `protocol.md` 一致
- 修复前端视频 Worker 错误地把 `Blob` 放入 `postMessage` transfer list 导致首帧崩溃
- 修复固件 SNTP 同步判断可能把负错误码误判为成功
- 修复前端 ConfigDialog 自动填充的 server 缺少 scheme 导致校验失败的问题：现在自动填充为 `http(s)://host:port`（默认端口补全 80/443），且 server 字段校验强制要求 `http://` 或 `https://` scheme
- 增加固件 `handleControl` 串口日志，便于确认 WASD 指令到达
- 修复固件视频任务在连续上传约 20 帧后触发 `CORRUPT HEAP: Bad head` 并断言 `multi_heap_free (head != NULL)` 的问题，同时改善视频帧率：`videoTask` 将 JPEG 复制到 PSRAM 后立即归还摄像头帧缓冲，避免网络 I/O 长时间占用 `camera_fb_t`；`httpsHandshakeFailed` 改用 `portMUX_TYPE` 自旋锁实现跨核原子读写
- 修复固件 HTTPS 模式下视频帧率极低（0.3-1fps）与 WASD 命令延时高（最坏 ~30s）的问题：视频通道成功路径不再每帧 `stop()` 客户端以复用 TLS 连接；`pollTask` 长轮询改用独立 `pollSecureClient`/`pollClient`，不再与 `loop()` 的 telemetry/event POST 共用 `httpsMutex`；`loop()` 中指令派发提前到遥测之前
- 修复 HTTP + WiFi 环境下 WASD 命令延时仍有 ~0.5s 的问题：将 `sendTelemetry()` 从 `loop()` 提取到独立 `telemetryTask`（core 0，独立客户端），`loop()` 不再包含任何同步网络 I/O，命令延时降到 <10ms

### Changed
- 固件 `logTlsAndHttpError` HTTPS 模式 TLS 失败措辞改善：从 `[TLS] <tag> lastErr=<n> <desc>` 改为 `[TLS] <tag> handshake failed: code=<n> <desc> (backend may be plain HTTP)`，明确指出可能 scheme 不匹配（而非误导的 `connection refused`）

### Note
- 后端 HTTP/2 h2c 已支持（`actix-http` `http2` feature + `tcp_auto_h2c()`），nginx 反代或 h2c 客户端可用 HTTP/2
- 固件受 ESP32 Arduino `HTTPClient` 库限制仍走 HTTP/1.1
- HTTP/3 (QUIC over UDP) 在 ESP32 上不可行，需大量移植工作，不在本版本范围

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
- 后端：新增 `POST /api/auth/login` 端点校验前端访问密码；配置文件 `auth.frontend_password` 字段（默认 `admin1234`，部署前修改）
- 前端：访问前端需密码验证，登录态存 sessionStorage（关 tab 重登）；新增 LoginView 组件与退出登录按钮
- 新增 GitHub Actions 自动构建（`.github/workflows/build-backend.yml`）：后端覆盖 7 个目标平台（Linux gnu/musl × x86_64/aarch64、Windows x86_64、macOS x86_64/aarch64）+ ESP32-S3 固件单个 .bin（arduino-cli 编译 + esptool 合并 bootloader/partitions/app，可直接烧录到 0x0）；产物上传至 GitHub Actions artifacts；触发条件为 push master（版本=`latest`）、push tag `v*`（版本=tag 名）、手动触发

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
- 固件：重映射 5 个冲突 GPIO（PIN_IN1=4→41, PIN_IN2=5→42, PIN_IN3=6→47, PIN_IN4=7→21, PIN_ENC_RIGHT=15→3），消除与摄像头 SCCB（SIOD/SIOC）/ VSYNC / HREF / XCLK 物理冲突，修复"无视频画面 + 拍照 SCCB_Write Failed ret=259"症状
- 后端：`PhotoResult` 新增 `ok: bool` 字段；`DeviceEvent::Error` 分支调 `complete_photo(ok=false)` 立即释放挂起的 photo oneshot；`/api/photo/{id}` 在设备侧失败时返回 HTTP 502 Bad Gateway（区别于超时 504）

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
