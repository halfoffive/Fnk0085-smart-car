# AGENTS.md — Fnk0085 智能小车

> 给 OpenCode 会话的精简工作须知。只记容易踩坑或与默认不同的点。

## 仓库结构

三端一体单仓，**版本号统一**（固件 / 后端 / 前端 / PWA 缓存键共用 `package.json` 的 `version`，改版本时 PWA 会自动清旧缓存）：

- `firmware/Fnk0085-smart-car/` — ESP32-S3 Arduino `.ino`（摄像头 + 电机 + 编码器 + PID + Web Serial 配网）。**控制平面走 HTTP/HTTPS 长轮询**（按 `server` scheme 选择 `WiFiClient` 或 `WiFiClientSecure::setInsecure()`；FreeRTOS `pollTask` 拉指令，`cmdQueue` 投递给 `loop` 派发）；视频帧通过 POST `/api/device/{id}/frame` 上传（`httpsPostFrame`，scheme-aware）；FreeRTOS Mutex 保护共享客户端；`WiFi.config()` 在 `WiFi.begin()` 前设 DNS `119.29.29.29` + `8.8.8.8`。**SD_MMC 必须先 `setPins(39, 38, 40)` 再 `begin("/sdcard", true, true, SDMMC_FREQ_DEFAULT, 5)`**（5 参数版），否则 ESP32-S3 默认引脚挂载失败。
- `backend/` — Rust + actix-http v3.13.1（**启用 `http2` feature 用于 h2c 协商**，明文 HTTP，回退 HTTP/1.1；ALPN / TLS 由 nginx 反代统一处理）。**设备控制通道为 HTTP/HTTPS 长轮询**：每设备 `mpsc::channel(16)` + `AsyncMutex<Receiver>`，`/api/device/{id}/register|poll|event`；设备视频帧入口为 `POST /api/device/{id}/frame`。`include_dir!` 编译期内嵌 `../frontend/dist`
- `frontend/` — Vite 8.1.0 + Vue 3.5.9 + TailwindCSS 4.3.1，包管理器用 **bun**（不再用 npm）。PWA 用 `injectManifest` + 自定义 `src/sw.ts`（**不要改回 `generateSW`**，曾经在 Vite 8 下因 workbox shim 异步加载导致 install handler 注册晚于 install 事件，离线白屏）
- `protocol.md` — 设备↔后端↔前端的通信契约，**任何字段变更必须同步改三端实现 + 本文档**

## 构建顺序（硬约束）

后端编译期通过 `backend/build.rs` **自动构建前端**：检测 `node_modules` 缺失则 `bun install`，检测 `dist/` 缺失或源码（`src/`、`vite.config.ts`、`tsconfig*`、`index.html`、`package.json`）较新则 `bun run build`，再 `include_dir!` 内嵌。所以日常只需：

```
cd backend
cargo build --release      # build.rs 自动跑 bun install / bun run build
```

手工构建前端（仅开发预览 / 调试 build 产物）：

```
cd frontend
bun install
bun run build              # = vue-tsc -b && vite build；产物到 dist/
bun run dev                # 仅开发预览，不更新后端内嵌产物
```

**注意**：build.rs 走的是 `bun.exe`（Windows）/ `bun`（其它平台），需 bun 在 PATH 中。改前端源码后直接 `cargo build`，build.rs 会按 mtime 自动重建 dist。

## CI/CD

`.github/workflows/build-backend.yml` — GitHub Actions 自动构建后端二进制，覆盖 7 个目标平台（Linux gnu/musl × x86_64/aarch64、Windows x86_64、macOS x86_64/aarch64），产物上传至 GitHub Actions artifacts。

- **触发**：push `master`（版本=`latest`）、push tag `v*`（版本=tag 名）、`workflow_dispatch` 手动触发
- **构建策略**：原生 `cargo build --target` + 系统交叉编译工具链（不用 `cross`/Docker，因项目无 native C 依赖，且 build.rs 需访问 `../frontend`）
- **交叉编译工具链**：Linux x86_64-musl 用 `musl-tools`（apt），aarch64-gnu 用 `gcc-aarch64-linux-gnu`（apt），aarch64-musl 用 musl.cc 工具链（wget）
- **每个 job 先用 bun 预构建前端**，build.rs 的 mtime 检查会跳过重复构建
- **产物**：打包为 `fnk0085-smart-car-backend-<version>-<target>[.tar.gz|.zip]`，上传至 GitHub Actions artifacts（保留 30 天）
- **改 workflow 后无需跑 cargo**（纯 YAML），但需确认 YAML 语法正确

## Windows 环境坑

- 工作目录路径含中文，可能导致 Rust build script panic：设 `$env:CARGO_TARGET_DIR = "C:\temp\fnk0085-target"` 绕过。
- `cargo build` 可能因 `version_check` crate spawn `rustc --version` 报 `Os { code: 0 }` 失败（Windows Defender 拦 build script 子进程的已知问题），与项目代码无关。复验请用 Linux / WSL2 / 关闭 Defender 对 `target/` 的实时扫描。
- 测试若遇 `net::ERR_EMPTY_RESPONSE`，清理残留 `python -m http.server` 进程并换端口（避免 TIME_WAIT）。

## 测试

- `tests/ui_smoke.py` — Playwright 同步 UI 烟雾 + PWA 离线验证。需先有 `frontend/dist`（`bun run build` 或直接 `cargo build` 触发 build.rs 自动产出）。
- 跑法：借助 `webapp-testing` skill 的 `with_server.py` 启静态服务器托管 `frontend/dist`，再跑 `tests/ui_smoke.py`。脚本内 `BASE_URL` 由 `UI_SMOKE_HOST` / `UI_SMOKE_PORT` 环境变量覆盖（默认 127.0.0.1:5180）。
- Playwright 依赖本地化在 `tests/.pylibs` 与 `tests/.browsers`（已 gitignore），通过 `PYTHONPATH` + `PLAYWRIGHT_BROWSERS_PATH` 复用，**不要 `pip install` 或 `playwright install`**。
- 测试无独立后端集成项；`cargo run` 行为靠真实环境复验（见 Windows 坑）。
- 截图/结果落盘到 `test-screenshots/` 和 `tests/results.txt`（gitignore，不入库）。

## 配置与密钥

- `*.jsonc` 均 gitignore（含 token）。**不要提交运行时生成的 config**。后端不再生成 `certs/`（TLS 由 nginx 反代处理，无内置证书）。
- 默认后端监听明文 HTTP 8080（h2c 协商 HTTP/2，回退 HTTP/1.1）；token 默认 `change-me-please`，部署前务必修改。
- **后端对外暴露**：仅监听内网端口，对外由 nginx 反代终止 TLS（HTTP/2 over TLS + HTTP/3 over QUIC），反代到 `127.0.0.1:8080`。nginx 配置示例见 [README.md](README.md) §2。
- **设备侧 HTTP/HTTPS 双模式**：根据 `server` 配置字段 scheme（`http://` 或 `https://`）选择客户端：
  - `http://` → `WiFiClient`（明文，直连后端内网端口，延时最低）
  - `https://` → `WiFiClientSecure` + `setInsecure()`（信任所有证书，开发期方案；生产由 nginx 提供合法证书，固件无需感知 CA 轮换）
  - 兼容无 scheme 老配置：默认走 HTTPS
- **固件 DNS**：`WiFi.config(INADDR_NONE, INADDR_NONE, INADDR_NONE, IPAddress(119,29,29,29), IPAddress(8,8,8,8))` 在 `WiFi.begin()` 前调用（5 参数版：DHCP 自动获取 IP/Gateway/Subnet，仅显式指定 DNS1+DNS2）。`119.29.29.29` 是 DNSPod 国内 DNS，`8.8.8.8` 是 Google 备份。
- 设备首帧走 `POST /api/device/{id}/register`（body 携带 token），其后所有端点带 `Authorization: Bearer <token>` 头。视频帧 POST body 为原始 JPEG（`Content-Type: image/jpeg`，header 另带 `X-Device-Uptime-Ms`）。
- **前端访问密码**：配置文件 `auth.frontend_password` 字段（明文存 jsonc，默认 `admin1234`），前端访问需 `POST /api/auth/login` 校验通过后存 sessionStorage（关 tab 重登）。**仅页面访问门槛**，不保护前端面向 API（control/photo/stream 等仍无鉴权，知道后端地址可直调），生产安全依赖 nginx 网络层隔离。
- SNTP 时间同步（`configTime(0, "pool.ntp.org", "time.google.com")`，5s 超时失败仅告警）保留作为日志辅助；`setInsecure()` 跳过证书时间校验，SNTP 失败不影响 HTTPS 握手。
- HTTP 协议版本：后端单端口明文 HTTP/2（h2c），HTTP/3 由 nginx 反代提供（actix-http 不支持 HTTP/3）。
- 固件串口配网等待期发送单行 `CONFIG\n`（不带字段）可查询 NVS 中已存的 ssid / password 长度 / server / token；正常配网行格式见 `protocol.md` 第 4 节。
- 前端 ConfigDialog 提交前对 `server` 字段做合规校验（须带 `http://` 或 `https://` scheme + hostname / IPv4 / `[IPv6]` + `:port`，端口范围 1-65535），非法时阻止提交。

## Troubleshooting

- **固件日志 `[NET] register failed code=-1 resp=` + 后端日志 `actix_http::h1::dispatcher stream error: request parse error: invalid Header provided`**：根因是 NVS 中 `server=https://<host>:<port>` 与后端明文 HTTP 不匹配，固件发 TLS ClientHello 到明文端口。三种解决方式：
  1. **重新配网**：通过 Web Serial 下发 `CONFIG|...|server=http://<host>:<port>|...`，直连后端明文端口（最低延时，开发推荐）
  2. **部署 nginx 反代**：nginx 终止 TLS 后反代到后端明文端口，固件 NVS 改 `server=https://<域名>`（生产推荐）
  3. **固件已支持自动回退**：首次 TLS 握手失败（`HTTPClient` 返回 `-1`）后，固件自动用明文 `plainClient` 重试一次并置 `httpsHandshakeFailed = true`，后续请求 session 内 sticky 走明文；日志会打印 `[TLS] <tag> lastErr=<code> <buf>` 与 `[HTTP] <tag> error=-1 <errorToString>` 帮助诊断；NVS 重配或重启后复位
- **固件日志 `[TLS] lastErr=-0x2700 ...`**：mbedTLS 错误码 `-0x2700` 是 `MBEDTLS_ERR_SSL_FATAL_ALERT_MESSAGE`，常见于对端拒绝 TLS（如对端是明文 HTTP 服务）；`-0x7780` 是 `MBEDTLS_ERR_SSL_CONN_EOF`（对端关闭连接）。完整码表见 `mbedtls/error.h`
- **前端 devtools 报 `404 {"deviceId":"%E2%97%8F%20online","error":"device not found"}`**：URL 解码 `%E2%97%8F%20online` = `● online`，根因是 `DeviceSelect.vue` 的 `<option>` textContent 附加了状态指示符 `● online` / `○ offline` 后缀，浏览器对 `<option :value>` 的边缘处理 fallback 到 textContent 时把整串 `● online` 当作 deviceId 提取。已修复：option textContent 简化为只显示 `d.deviceId`，状态指示符仅由 selectedDevice 状态条渲染；`frontend/src/lib/validate.ts` 的 `isValidDeviceId` 在 4 个 API 函数（postControl / postPhoto / getPwmCache / postPwmCache）+ videoWorker start 入口拦截非法 deviceId，非法时直接 throw 不发请求。
- **固件日志 `[BOOT] setup done` 后立即出现 `ESP-ROM:esp32s3-20210327 / rst:0x1 (POWERON)`**：根因是 videoTask / pollTask 栈溢出（8192 字节不够 HTTPClient + WiFiClientSecure + TLS 握手栈占用）。已修复：`POLL_TASK_STACK` 与新增 `VIDEO_TASK_STACK` 常量均提升到 16384 字节；任务入口打印 `[TASK] video/poll started` 心跳；setup 末尾 `[BOOT] setup done` 之后校验任务句柄非 NULL，失败时打印 `[BOOT] task create failed, rebooting in 5s` 并延时重启，成功时打印 `[BOOT] tasks launched`。
- 固件启动主动探测后端 scheme：WiFi 关联 + SNTP 同步后、首次 register 之前调用 `probeScheme()`，先探测 NVS 配的 scheme（GET /api/health），失败则试另一 scheme，成功则切换 `useHttps` + 写回 NVS（auto-correct），探测期间静默 TLS 错误。冷启动日志应只出现 `[NET] probe: <scheme> ok, using <scheme>` 或 `[NET] probe: <old> failed (code=<n>), <new> ok, switching to <new> (NVS updated)`
- 后端 `/api/health` 端点：`GET /api/health` 无鉴权，返回 200 + `{"status":"ok","version":"0.3.1"}`，用于固件启动 scheme 探测与运维监控
- 后端 HTTP/2 h2c 支持情况：`actix-http` 启用 `http2` feature + `tcp_auto_h2c()` 协商明文 HTTP/2，nginx 反代或 h2c 客户端可用 HTTP/2；固件受 ESP32 Arduino `HTTPClient` 库限制仍走 HTTP/1.1；HTTP/3 (QUIC) 在 ESP32 上不可行
- **若前端设备名为空且视频一直 loading**：先检查 `GET /api/devices` 响应字段是否为 camelCase（`deviceId`/`lastSeenMs`）；排查工具可用浏览器 devtools Network 查看响应字段，若仍为 `device_id`/`last_seen_ms` 说明后端序列化未生效
- **若 S3 串口按 WASD 无反应**：先确认串口已输出 `[CTRL] direction=... pwm=... durationMs=...`；若无此日志，则指令未到达固件，需依次检查前端 deviceId 是否合法、后端 `/api/device/{id}/poll` 队列是否正常、固件 `pollTask` 是否在线
- **拍照时串口报 `SCCB_Write Failed addr:0x30 ... ret:259` 且后端 504**：根因是 GPIO 引脚冲突（PIN_IN1=4 与 SIOD 冲突、PIN_IN2=5 与 SIOC 冲突、PIN_IN3=6 与 VSYNC 冲突、PIN_IN4=7 与 HREF 冲突、PIN_ENC_RIGHT=15 与 XCLK 冲突），`motorInit()` / `encoderInit()` 在 `cameraInit()` 之后执行把摄像头 GPIO 全部重路由掉。已修复：电机方向引脚重映射到 41/42/47/21，右编码器移到 GPIO 3（strapping pin，boot 后 INPUT_PULLUP 安全）。`photoMux` 二进制信号量仍保留作为 videoTask / handlePhoto 的并发保护（次要但合理）。拍照失败时设备上报 `DeviceEvent::Error`，后端立即触发 photo oneshot 释放，`/api/photo/{id}` 返回 HTTP 502 而非等满 8s 返回 504。
- **前端视频画面无流**：检查串口是否周期打印 `[VIDEO] frames ok=%u fail=%u` 与 `[FRAME] POST failed ...`；`ok=0 fail=0` 表示 videoTask 未拿到帧（可能 photoInProgress 卡住或摄像头初始化失败）；`fail` 持续增加则检查后端 `/api/device/{id}/frame` 是否返回 401/404/413 或 scheme 是否匹配。
- **忘记前端密码**：编辑运行目录下 `Fnk0085-smart-car-config.jsonc` 的 `auth.frontend_password` 字段后重启后端；若配置文件不存在，后端启动会自动写入默认值 `admin1234`。

## 协议改动清单

改 `protocol.md` 任意字段时，按清单同步：`firmware/Fnk0085-smart-car/*.ino`、`backend/src/protocol.rs` 与各 handler（`device_api.rs` / `frame.rs` / `telemetry.rs` / `provisioning.rs`）、`frontend/src/**`、`protocol.md`。视频帧走 POST（`/api/device/{id}/frame`），body 原始 JPEG，header `Authorization` / `Content-Type` / `X-Device-Uptime-Ms`；协议版本 3。HTTP/HTTPS 设备指令走 `DeviceCommand` enum（tag=`type`，字段 camelCase），事件走 `DeviceEvent` enum（同命名约定），新增 variant 时三端同步。新增设备端点（如 `GET /api/telemetry/{id}`、`GET /api/config`）或修改访问日志格式时，同步更新 `README.md` 后端 API 速查表。

## 提交约定

- 提交信息用中文。
- 仅提交与本任务直接相关的文件，避免捎带运行时产物（`*.jsonc`、`certs/`、`dist/`、`node_modules/`、`target/`、`test-screenshots/`、`.trae/` 等）。
- 真要改版本号，三端 + `CHANGELOG.md` + PWA 缓存键一起动。


## 额外要求

- 修改遵循本文件
- 代码：函数式编程，适量中文注释，写的代码要方便维护。
- 完成修改时：rust 部分`cargo fmt`统一格式，然后`cargo clippy`、`cargo test`和`cargo build`全过方可提交。然后更新 `README.md` 、 `CHANGELOG.md` 和 `AGENTS.md`。最后提交git并推送。
- 规范：```


Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
```