# Changelog

本项目版本号统一：固件 / 后端 / 前端共享 `0.1.0`。前端 PWA 缓存键带版本号，版本变更时自动清理旧缓存。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

### Changed
- 前端框架由 React 18 迁移至 Vue 3.5.9（SFC + `<script setup lang="ts">` + composables），包管理器由 npm 切换为 **bun**
- 后端新增 `backend/build.rs`：编译期自动检测 `frontend/node_modules` 与 `frontend/dist`，按需执行 `bun install` / `bun run build`，再 `include_dir!` 内嵌；日常 `cargo build` 即可

### Fixed
- 修复 Chrome 访问 HTTPS 端口报 `ERR_CONNECTION_RESET` 的问题：
  - 根因：actix-http `rustls_0_23` acceptor 强制把 `h2` 注入 ALPN 列表（service.rs:444-446），覆盖我们设置的 `["http/1.1"]`，Chrome 协商到 h2 后 actix-http 因未启用 `http2` feature panic
  - 修复：启用 `actix-http` 的 `http2` feature，同时移除无效的 `alpn_protocols = ["http/1.1"]` 设置（actix-http 默认注入 `[h2, http/1.1]`，浏览器自动协商）
- 修复固件在 ESP32 Arduino 核心 3.3.8-cn 下的编译错误：
  - `ledcSetup` / `ledcAttachPin` 已废弃 → 改用 `ledcAttach(pin, freq, res)` + `ledcWrite(pin, duty)`
  - `ESP.getChipId()` 不再存在 → 改用 `ESP.getEfuseMac()`
  - `mbedtls_sha256_starts_ret` / `_update_ret` / `_finish_ret` 已合并为无后缀版本
  - `volatile ++` 触发 `-Wvolatile` → 改为显式 `x = x + 1`
  - 结构体聚合初始化缺少成员告警 → 显式列出全部字段

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
