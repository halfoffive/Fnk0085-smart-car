"""
Fnk0085 智能小车集成测试 — UI 烟雾测试 (Task 5)

依赖：
- Playwright (PYTHONPATH 指向 frontend/tests/.pylibs)
- Chromium (PLAYWRIGHT_BROWSERS_PATH 指向 frontend/tests/.browsers)

运行方式（通过 webapp-testing skill 的 with_server.py 启动静态服务器）：
    python scripts/with_server.py \
        --server "python -m http.server 5173 --directory frontend/dist" \
        --port 5173 \
        -- python tests/ui_smoke.py

测试内容：
- 测试 1：UI 元素验证（页面加载、设备选择、WASD、调速滑块、拍照、配网、PWM 缓存开关）
- 测试 3：PWA 离线刷新不白屏（Service Worker 缓存）
"""

from __future__ import annotations

import os
import sys
import re
from pathlib import Path
from typing import List

from playwright.sync_api import (
    BrowserContext,
    Page,
    Playwright,
    expect,
    sync_playwright,
)

# ===== 配置 =====
_PORT = os.environ.get("UI_SMOKE_PORT", "5180")
_HOST = os.environ.get("UI_SMOKE_HOST", "127.0.0.1")
BASE_URL = f"http://{_HOST}:{_PORT}"
SCREENSHOT_DIR = Path(r"d:\工作目录\Fnk0085-smart-car\test-screenshots")
RESULTS_FILE = Path(r"d:\工作目录\Fnk0085-smart-car\tests\results.txt")

SCREENSHOT_DIR.mkdir(parents=True, exist_ok=True)
RESULTS_FILE.parent.mkdir(parents=True, exist_ok=True)


# ===== 测试结果收集 =====
class _R:
    def __init__(self) -> None:
        self.items: List[dict] = []

    def add(self, name: str, passed: bool, detail: str = "") -> None:
        self.items.append({"name": name, "passed": passed, "detail": detail})
        tag = "PASS" if passed else "FAIL"
        print(f"[{tag}] {name}: {detail}")

    @property
    def passed_count(self) -> int:
        return sum(1 for r in self.items if r["passed"])

    @property
    def failed_count(self) -> int:
        return sum(1 for r in self.items if not r["passed"])


R = _R()


# ===== 测试 1：UI 元素 =====
def test_ui_elements(page: Page) -> None:
    print("\n=== Test 1: UI Elements ===")

    # 1. 页面加载 + 标题
    try:
        page.goto(BASE_URL + "/", wait_until="networkidle", timeout=20000)
        title = page.title()
        R.add("page_load", True, f"title='{title}'")
    except Exception as e:  # pragma: no cover
        R.add("page_load", False, f"goto failed: {e}")
        return

    title_ok = "Fnk0085" in title or "fnk0085" in title.lower()
    R.add("title_contains_fnk0085", title_ok, title)

    # 2. 设备选择下拉框
    try:
        device_select = page.locator("select").first
        expect(device_select).to_be_visible(timeout=5000)
        # 验证有占位 option
        first_option_text = device_select.locator("option").first.inner_text()
        R.add(
            "device_select_visible",
            True,
            f"first option='{first_option_text.strip()}'",
        )
    except Exception as e:  # pragma: no cover
        R.add("device_select_visible", False, str(e))

    # 3. WASD 4 个按钮 — 通过子标签文字 (fwd/left/back/right) 精确定位
    wasd_subs = {"W": "fwd", "A": "left", "S": "back", "D": "right"}
    for letter, sub in wasd_subs.items():
        try:
            btn = page.locator(f"button:has-text('{sub}')").first
            expect(btn).to_be_visible(timeout=3000)
            R.add(f"wasd_button_{letter}", True, f"sub='{sub}' visible")
        except Exception as e:  # pragma: no cover
            R.add(f"wasd_button_{letter}", False, str(e))

    # 4. 1-100 调速滑块
    try:
        slider = page.locator("input[type='range']").first
        expect(slider).to_be_visible(timeout=3000)
        min_val = slider.get_attribute("min")
        max_val = slider.get_attribute("max")
        ok = min_val == "1" and max_val == "100"
        R.add("throttle_slider_1_100", ok, f"min={min_val}, max={max_val}")
    except Exception as e:  # pragma: no cover
        R.add("throttle_slider_1_100", False, str(e))

    # 5. 拍照按钮 — 文字 "capture photo"
    try:
        photo_btn = page.locator("button:has-text('capture photo')").first
        expect(photo_btn).to_be_visible(timeout=3000)
        R.add("photo_button", True, "capture photo visible")
    except Exception as e:  # pragma: no cover
        R.add("photo_button", False, str(e))

    # 6. 配网按钮 — 文字 "打开配网弹窗"
    try:
        config_btn = page.locator("button:has-text('打开配网弹窗')").first
        expect(config_btn).to_be_visible(timeout=3000)
        R.add("config_button", True, "打开配网弹窗 visible")
    except Exception as e:  # pragma: no cover
        R.add("config_button", False, str(e))

    # 7. PWM 缓存开关 — role=switch
    try:
        toggle = page.locator("button[role='switch']").first
        expect(toggle).to_be_visible(timeout=3000)
        R.add("pwm_cache_toggle", True, "role=switch visible")
    except Exception as e:  # pragma: no cover
        R.add("pwm_cache_toggle", False, str(e))

    # 截图
    try:
        page.screenshot(path=str(SCREENSHOT_DIR / "ui.png"), full_page=True)
        R.add("screenshot_ui", True, str(SCREENSHOT_DIR / "ui.png"))
    except Exception as e:  # pragma: no cover
        R.add("screenshot_ui", False, str(e))


# ===== 测试 3：PWA 离线 =====

# 全局收集 SW worker 的事件（在 main() 里绑定到 page.on("worker", ...)）
SW_WORKER_LOGS: List[str] = []


def test_offline_pwa(page: Page, context: BrowserContext) -> None:
    print("\n=== Test 3: PWA Offline (native SW) ===")

    # 0. 清理可能存在的旧 SW 注册（确保测的是新构建的 sw.js）
    try:
        unreg = page.evaluate(
            """async () => {
                const regs = await navigator.serviceWorker.getRegistrations();
                const out = [];
                for (const r of regs) {
                    out.push(r.scope);
                    await r.unregister();
                }
                // 清空所有 caches（避免旧 precache 干扰）
                const keys = await caches.keys();
                for (const k of keys) {
                    await caches.delete(k);
                }
                return {unregistered: out, clearedCaches: keys};
            }"""
        )
        print(f"pre-cleanup: {unreg}")
        # 清理后强制 reload，让新的 SW 干净注册
        page.goto(BASE_URL + "/", wait_until="networkidle", timeout=20000)
    except Exception as e:  # pragma: no cover
        print(f"pre-cleanup failed: {e}")

    # 1. 等待 navigator.serviceWorker.ready（resolve 表示有 active SW）
    # 先做一次诊断快照，看 SW 注册的实时状态
    try:
        sw_snap = page.evaluate(
            """async () => {
                const out = {
                    swInNavigator: ('serviceWorker' in navigator),
                    controller: navigator.serviceWorker.controller
                        ? {scriptURL: navigator.serviceWorker.controller.scriptURL, state: navigator.serviceWorker.controller.state}
                        : null,
                    registrations: []
                };
                try {
                    const regs = await navigator.serviceWorker.getRegistrations();
                    for (const r of regs) {
                        out.registrations.push({
                            scope: r.scope,
                            active: r.active ? {state: r.active.state, scriptURL: r.active.scriptURL} : null,
                            installing: r.installing ? {state: r.installing.state, scriptURL: r.installing.scriptURL} : null,
                            waiting: r.waiting ? {state: r.waiting.state, scriptURL: r.waiting.scriptURL} : null,
                        });
                    }
                } catch(e) { out.getRegsErr = String(e); }
                return out;
            }"""
        )
        print(f"sw_snap (before ready wait): {sw_snap}")
    except Exception as e:  # pragma: no cover
        print(f"sw_snap failed: {e}")

    # 1a. 若没注册，尝试显式注册一次（捕获 register() 的错误）
    try:
        manual_reg = page.evaluate(
            """async () => {
                try {
                    const reg = await navigator.serviceWorker.register('/sw.js', {scope: '/', type: 'classic'});
                    return {ok: true, scope: reg.scope};
                } catch(e) {
                    return {ok: false, err: String(e), name: e.name};
                }
            }"""
        )
        print(f"manual_register: {manual_reg}")
    except Exception as e:  # pragma: no cover
        print(f"manual_register evaluate failed: {e}")

    try:
        page.wait_for_function(
            """async () => {
                const reg = await navigator.serviceWorker.ready;
                return reg !== undefined && reg.active !== null;
            }""",
            timeout=20000,
        )
        R.add("sw_ready", True, "navigator.serviceWorker.ready resolved")
    except Exception as e:  # pragma: no cover
        R.add("sw_ready", False, str(e))

    # 1b. 强制 SW update + 监听 SW 控制台错误（捕获 install handler 异常）
    # 注意：navigator.serviceWorker.ready 在 SW install 失败时永不 resolve，
    # 用 Promise.race 加 5s 超时避免 evaluate 永久挂起。
    try:
        sw_diag = page.evaluate(
            """async () => {
                const readyOrTimeout = Promise.race([
                    navigator.serviceWorker.ready,
                    new Promise(resolve => setTimeout(() => resolve(null), 5000)),
                ]);
                const reg = await readyOrTimeout;
                if (!reg) return {readyTimeout: true};
                // 触发 update 检查（如果 sw.js 没变，会 no-op）
                try { await reg.update(); } catch(e) {}
                // 收集 SW 状态
                const out = {
                    scope: reg.scope,
                    active: reg.active ? {state: reg.active.state, scriptURL: reg.active.scriptURL} : null,
                    installing: reg.installing ? {state: reg.installing.state, scriptURL: reg.installing.scriptURL} : null,
                    waiting: reg.waiting ? {state: reg.waiting.state, scriptURL: reg.waiting.scriptURL} : null
                };
                return out;
            }"""
        )
        print(f"sw_diag after update: {sw_diag}")
    except Exception as e:  # pragma: no cover
        print(f"sw_diag failed: {e}")

    # 2. 等待 controller 设置
    try:
        page.wait_for_function(
            """() => navigator.serviceWorker.controller !== null""",
            timeout=10000,
        )
        R.add("sw_controller", True, "controller present")
    except Exception as e:  # pragma: no cover
        R.add("sw_controller", False, str(e))

    # 3. 等待 precache 完成 — caches.keys() 应包含以 fnk0085-v0.1.0 为前缀的 cache
    # workbox setCacheNameDetails({prefix:"fnk0085-v0.1.0"}) 生成 fnk0085-v0.1.0-precache-v2
    PRECACHE_PREFIX = "fnk0085-v0.1.0"
    # 用 Python 端轮询替代 wait_for_function，便于打印诊断信息
    cache_populated = False
    cache_diag = None
    poll_deadline = 20  # seconds
    poll_interval_ms = 500
    for _ in range(int(poll_deadline * 1000 / poll_interval_ms)):
        try:
            cache_diag = page.evaluate(
                """async () => {
                    const names = await caches.keys();
                    return {
                        names: names,
                        matched: names.filter(n => n.startsWith('fnk0085-v0.1.0')),
                        controller: navigator.serviceWorker.controller
                            ? navigator.serviceWorker.controller.scriptURL : null
                    };
                }"""
            )
        except Exception as e:  # pragma: no cover
            cache_diag = {"err": str(e)}
            break
        if cache_diag.get("matched"):
            cache_populated = True
            break
        page.wait_for_timeout(poll_interval_ms)

    if cache_populated:
        R.add(
            "sw_precache_populated",
            True,
            f"caches={cache_diag.get('names')}, matched={cache_diag.get('matched')}, "
            f"controller={cache_diag.get('controller')}",
        )
    else:
        # 最终诊断
        try:
            final_state = page.evaluate(
                """async () => {
                    const readyOrTimeout = Promise.race([
                        navigator.serviceWorker.ready,
                        new Promise(resolve => setTimeout(() => resolve(null), 5000)),
                    ]);
                    const reg = await readyOrTimeout;
                    if (!reg) {
                        return {
                            readyTimeout: true,
                            controllerURL: navigator.serviceWorker.controller
                                ? navigator.serviceWorker.controller.scriptURL : null,
                            cacheNames: await caches.keys()
                        };
                    }
                    return {
                        activeState: reg.active ? reg.active.state : null,
                        scope: reg.scope,
                        controllerURL: navigator.serviceWorker.controller
                            ? navigator.serviceWorker.controller.scriptURL : null,
                        cacheNames: await caches.keys()
                    };
                }"""
            )
        except Exception as e:  # pragma: no cover
            final_state = {"err": str(e)}
        R.add(
            "sw_precache_populated",
            False,
            f"caches empty after {poll_deadline}s polling; "
            f"final_diag={cache_diag}; sw_state={final_state}",
        )

    # 4. 收集 SW worker 错误日志（如果有）
    if SW_WORKER_LOGS:
        joined = " | ".join(SW_WORKER_LOGS[:8])
        R.add("sw_worker_logs", True, joined[:500])
    else:
        R.add("sw_worker_logs", True, "(no SW worker console/error events captured)")

    # 5. 离线刷新测试 — 原生 SW：set_offline(True) + reload
    #    SW NavigationRoute 应返回缓存的 index.html，页面不白屏
    context.set_offline(True)
    print("context.set_offline(True) (via=native SW)")

    try:
        # 离线刷新
        reload_ok = False
        reload_err = ""
        try:
            page.reload(wait_until="commit", timeout=15000)
            reload_ok = True
        except Exception as e:
            reload_err = str(e)

        if not reload_ok:
            print(f"reload with wait_until='commit' failed: {reload_err}")
            # 退而求其次：尝试 goto
            try:
                page.goto(BASE_URL + "/", wait_until="commit", timeout=15000)
                reload_ok = True
                R.add(
                    "offline_reload",
                    True,
                    "via goto (commit) [native SW]",
                )
            except Exception as e2:
                R.add(
                    "offline_reload",
                    False,
                    f"reload and goto both failed [native SW]: "
                    f"{reload_err} | {e2}",
                )
        else:
            R.add(
                "offline_reload",
                True,
                "reload commit ok [native SW]",
            )

        if not reload_ok:
            # 离线刷新失败 — 截图（捕获 chrome-error 页或空白页）
            try:
                page.screenshot(
                    path=str(SCREENSHOT_DIR / "offline_fixed.png"),
                    full_page=True,
                )
                R.add(
                    "screenshot_offline",
                    True,
                    f"(failure state) {SCREENSHOT_DIR / 'offline_fixed.png'}",
                )
            except Exception as e:  # pragma: no cover
                R.add("screenshot_offline", False, str(e))
            return

        # 给渲染时间 — 等待 React hydrate + workbox 在离线下从 precache 提供资源
        page.wait_for_timeout(2500)

        # 检查 #root 是否有内容
        try:
            root_html = page.locator("#root").inner_html()
            not_blank = len(root_html.strip()) > 200
            R.add(
                "offline_not_blank",
                not_blank,
                f"#root innerHTML length={len(root_html)}",
            )
        except Exception as e:  # pragma: no cover
            R.add("offline_not_blank", False, str(e))

        # 检查标题仍含 Fnk0085
        try:
            title = page.title()
            title_ok = "Fnk0085" in title or "fnk0085" in title.lower()
            R.add("offline_title", title_ok, f"title='{title}'")
        except Exception as e:  # pragma: no cover
            R.add("offline_title", False, str(e))

        # 检查品牌标题仍可见
        try:
            brand = page.locator("text=Fnk0085").first
            expect(brand).to_be_visible(timeout=3000)
            R.add("offline_brand_visible", True, "Fnk0085 brand visible")
        except Exception as e:  # pragma: no cover
            R.add("offline_brand_visible", False, str(e))

        # 检查关键控件（设备选择）仍可见
        try:
            expect(page.locator("select").first).to_be_visible(timeout=3000)
            R.add("offline_device_select", True, "select still visible offline")
        except Exception as e:  # pragma: no cover
            R.add("offline_device_select", False, str(e))

        # 检查 WASD 按钮仍可见
        for letter, sub in {"W": "fwd", "A": "left", "S": "back", "D": "right"}.items():
            try:
                btn = page.locator(f"button:has-text('{sub}')").first
                expect(btn).to_be_visible(timeout=3000)
                R.add(f"offline_wasd_{letter}", True, f"sub='{sub}' visible")
            except Exception as e:  # pragma: no cover
                R.add(f"offline_wasd_{letter}", False, str(e))

        # 检查调速滑块仍可见
        try:
            slider = page.locator("input[type='range']").first
            expect(slider).to_be_visible(timeout=3000)
            R.add("offline_slider", True, "throttle slider visible")
        except Exception as e:  # pragma: no cover
            R.add("offline_slider", False, str(e))

        # 截图
        try:
            page.screenshot(
                path=str(SCREENSHOT_DIR / "offline_fixed.png"),
                full_page=True,
            )
            R.add(
                "screenshot_offline",
                True,
                str(SCREENSHOT_DIR / "offline_fixed.png"),
            )
        except Exception as e:  # pragma: no cover
            R.add("screenshot_offline", False, str(e))
    finally:
        context.set_offline(False)


# ===== 主入口 =====
def main() -> int:
    print("Fnk0085 Smart Car - UI Smoke Test")
    print(f"BASE_URL: {BASE_URL}")
    print(f"SCREENSHOT_DIR: {SCREENSHOT_DIR}")

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(viewport={"width": 1440, "height": 900})
        page = context.new_page()

        # 收集 console 错误（仅信息用途）
        console_msgs: List[str] = []

        def _on_console(msg) -> None:
            if msg.type == "error":
                console_msgs.append(f"[page console.error] {msg.text}")

        page.on("console", _on_console)

        # 捕获 pageerror（未处理异常）
        def _on_pageerror(err) -> None:
            console_msgs.append(f"[page pageerror] {err}")

        page.on("pageerror", _on_pageerror)

        # 捕获 SW worker 的 console / error 事件
        def _on_worker(worker) -> None:
            def _w_console(msg) -> None:
                SW_WORKER_LOGS.append(f"[sw console.{msg.type}] {msg.text}")

            def _w_error(err) -> None:
                SW_WORKER_LOGS.append(f"[sw worker.error] {err}")

            worker.on("console", _w_console)
            worker.on("error", _w_error)

        page.on("worker", _on_worker)

        # 捕获 Service Worker 的 console / error 事件（与 web worker 不同）
        def _on_serviceworker(worker) -> None:
            def _sw_console(msg) -> None:
                SW_WORKER_LOGS.append(f"[serviceworker console.{msg.type}] {msg.text}")

            def _sw_error(err) -> None:
                SW_WORKER_LOGS.append(f"[serviceworker error] {err}")

            worker.on("console", _sw_console)
            worker.on("error", _sw_error)

        context.on("serviceworker", _on_serviceworker)

        try:
            test_ui_elements(page)
            test_offline_pwa(page, context)
        finally:
            browser.close()

    # 写入结果文件供 report 生成使用
    with open(RESULTS_FILE, "w", encoding="utf-8") as f:
        for r in R.items:
            tag = "PASS" if r["passed"] else "FAIL"
            detail = r["detail"].replace("\n", " ").replace("\t", " ")
            f.write(f"{tag}\t{r['name']}\t{detail}\n")
        f.write(f"\n# console errors (informational):\n")
        for m in console_msgs:
            f.write(f"CONSOLE_ERR\t{m}\n")

    print("\n=== Summary ===")
    print(f"PASS: {R.passed_count}, FAIL: {R.failed_count}, TOTAL: {len(R.items)}")
    if R.failed_count:
        print("\nFailed:")
        for r in R.items:
            if not r["passed"]:
                print(f"  - {r['name']}: {r['detail']}")

    return 0 if R.failed_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
