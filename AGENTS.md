# AGENTS.md — Fnk0085 智能小车

> 给 OpenCode 会话的精简工作须知。只记容易踩坑或与默认不同的点。

## 仓库结构

三端一体单仓，**版本号统一**（固件 / 后端 / 前端 / PWA 缓存键共用 `package.json` 的 `version`，改版本时 PWA 会自动清旧缓存）：

- `firmware/Fnk0085-smart-car/` — ESP32-S3 Arduino `.ino`（摄像头 + 电机 + 编码器 + PID + Web Serial 配网）
- `backend/` — Rust + actix-http v3.13.1（非 actix-web），多设备 UDP 转发 + 1s 视频环形缓存，`include_dir!` 编译期内嵌 `../frontend/dist`
- `frontend/` — Vite 8.1.0 + React 18 + TailwindCSS 4.3.1，PWA 用 `injectManifest` + 自定义 `src/sw.ts`（**不要改回 `generateSW`**，曾经在 Vite 8 下因 workbox shim 异步加载导致 install handler 注册晚于 install 事件，离线白屏）
- `protocol.md` — 设备↔后端↔前端的通信契约，**任何字段变更必须同步改三端实现 + 本文档**

## 构建顺序（硬约束）

后端编译期通过 `backend/build.rs` 断言 `../frontend/dist` 存在，并 `include_dir!` 内嵌。所以：

```
frontend: npm install && npm run build   # 先产 dist/
backend:  cargo run --release            # 再编译后端
```

改了前端不重新 `npm run build`，后端 `cargo build` 会 panic 或内嵌旧产物。

## 常用命令

```powershell
# 前端
cd frontend
npm install
npm run build      # = tsc -b && vite build；产物到 dist/
npm run dev        # 仅开发预览，不更新后端内嵌产物

# 后端
cd backend
cargo run --release
# 首次启动在「运行目录」生成 Fnk0085-smart-car-config.jsonc（端口/token/TLS 路径，JSONC 注释）和 certs/（rcgen 自签）。改配置后重启。
```

`npm run build` 不存在专属 lint/typecheck 脚本；类型检查由 `build` 内的 `tsc -b` 承担。改完前端只跑 `tsc -b` 也可快速验类型。

## Windows 环境坑

- 工作目录路径含中文，可能导致 Rust build script panic：设 `$env:CARGO_TARGET_DIR = "C:\temp\fnk0085-target"` 绕过。
- `cargo build` 可能因 `version_check` crate spawn `rustc --version` 报 `Os { code: 0 }` 失败（Windows Defender 拦 build script 子进程的已知问题），与项目代码无关。复验请用 Linux / WSL2 / 关闭 Defender 对 `target/` 的实时扫描。
- 测试若遇 `net::ERR_EMPTY_RESPONSE`，清理残留 `python -m http.server` 进程并换端口（避免 TIME_WAIT）。

## 测试

- `tests/ui_smoke.py` — Playwright 同步 UI 烟雾 + PWA 离线验证。需先 `npm run build` 产出 `frontend/dist`。
- 跑法：借助 `webapp-testing` skill 的 `with_server.py` 启静态服务器托管 `frontend/dist`，再跑 `tests/ui_smoke.py`。脚本内 `BASE_URL` 由 `UI_SMOKE_HOST` / `UI_SMOKE_PORT` 环境变量覆盖（默认 127.0.0.1:5180）。
- Playwright 依赖本地化在 `tests/.pylibs` 与 `tests/.browsers`（已 gitignore），通过 `PYTHONPATH` + `PLAYWRIGHT_BROWSERS_PATH` 复用，**不要 `pip install` 或 `playwright install`**。
- 测试无独立后端集成项；`cargo run` 行为靠真实环境复验（见 Windows 坑）。
- 截图/结果落盘到 `test-screenshots/` 和 `tests/results.txt`（gitignore，不入库）。

## 配置与密钥

- `*.jsonc`、`*.crt`、`*.key`、`*.pem` 均 gitignore（含 token 与证书路径）。**不要提交运行时生成的 config / certs**。
- 默认后端监听 HTTP 8080、UDP 7000；token 默认 `change-me-please`，部署前务必修改。
- UDP 视频走 AES-128-GCM AEAD（Arduino-ESP32 无 DTLS 高层 API，用 AEAD 等价替代）；HTTP 走 rustls 0.23，支持 mTLS（`client_ca` 设非 null 即启用）。

## 协议改动清单

改 `protocol.md` 任意字段时，按清单同步：`firmware/Fnk0085-smart-car/*.ino`、`backend/src/protocol.rs` 与各 handler、`frontend/src/**`、`protocol.md`。多字节整数小端序。UDP 分包固定 8 段、magic `0xF1 0xD0`、version 起 `1`。

## 提交约定

- 提交信息用中文。
- 仅提交与本任务直接相关的文件，避免捎带运行时产物（`*.jsonc`、`certs/`、`dist/`、`node_modules/`、`target/`、`test-screenshots/`、`.trae/` 等）。
- 真要改版本号，三端 + `CHANGELOG.md` + PWA 缓存键一起动。