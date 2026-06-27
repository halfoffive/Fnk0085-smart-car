# AGENTS.md — Fnk0085 智能小车

> 给 OpenCode 会话的精简工作须知。只记容易踩坑或与默认不同的点。

## 仓库结构

三端一体单仓，**版本号统一**（固件 / 后端 / 前端 / PWA 缓存键共用 `package.json` 的 `version`，改版本时 PWA 会自动清旧缓存）：

- `firmware/Fnk0085-smart-car/` — ESP32-S3 Arduino `.ino`（摄像头 + 电机 + 编码器 + PID + Web Serial 配网）
- `backend/` — Rust + actix-http v3.13.1（非 actix-web），多设备 UDP 转发 + 1s 视频环形缓存，`include_dir!` 编译期内嵌 `../frontend/dist`
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

- `*.jsonc`、`*.crt`、`*.key`、`*.pem` 均 gitignore（含 token 与证书路径）。**不要提交运行时生成的 config / certs**。
- 默认后端监听 HTTP 8080、UDP 7000；token 默认 `change-me-please`，部署前务必修改。
- UDP 视频走 AES-128-GCM AEAD（Arduino-ESP32 无 DTLS 高层 API，用 AEAD 等价替代）；HTTP 走 rustls 0.23，支持 mTLS（`client_ca` 设非 null 即启用）。

## 协议改动清单

改 `protocol.md` 任意字段时，按清单同步：`firmware/Fnk0085-smart-car/*.ino`、`backend/src/protocol.rs` 与各 handler、`frontend/src/**`、`protocol.md`。多字节整数小端序。UDP 分包固定 8 段、magic `0xF1 0xD0`、version 起 `1`。

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