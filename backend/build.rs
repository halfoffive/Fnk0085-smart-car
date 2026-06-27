//! 编译期自动构建前端：检测 dist/ 是否过期，必要时调用 bun 重建。
//!
//! 流程：
//! 1. 定位 `../frontend` 目录
//! 2. `node_modules` 缺失 → `bun install`
//! 3. `dist/` 缺失或 `src/` 比较新 → `bun run build`
//! 4. 失败则 panic，给出可读错误
//!
//! 由于 `include_dir!` 在路径不存在时无法直接给出友好提示，
//! 这里通过 build.rs 提前断言并自动修复。

use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

fn main() {
    let frontend_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("frontend");
    let dist_dir = frontend_dir.join("dist");
    let node_modules = frontend_dir.join("node_modules");
    let src_dir = frontend_dir.join("src");
    let pkg_json = frontend_dir.join("package.json");

    println!("cargo:rerun-if-changed=../frontend/package.json");
    println!("cargo:rerun-if-changed=../frontend/src");
    println!("cargo:rerun-if-changed=../frontend/index.html");
    println!("cargo:rerun-if-changed=../frontend/vite.config.ts");
    println!("cargo:rerun-if-changed=../frontend/tsconfig.json");
    println!("cargo:rerun-if-changed=../frontend/tsconfig.app.json");
    println!("cargo:rerun-if-changed=../frontend/tsconfig.node.json");

    // 1. node_modules 缺失 → bun install
    if !node_modules.is_dir() {
        run_bun(&frontend_dir, &["install"], "bun install");
    }

    // 2. dist 缺失或源码更新 → bun run build
    if !is_dist_valid(&dist_dir, &src_dir, &pkg_json, &frontend_dir) {
        run_bun(&frontend_dir, &["run", "build"], "bun run build");
    }

    // dist 已就绪 → 重编 include_dir 内容
    println!("cargo:rerun-if-changed=../frontend/dist");
}

/// 判断 dist/ 是否最新：缺失或源码（src/、index.html、vite.config.ts、tsconfig*、package.json）较新则需重建。
fn is_dist_valid(dist: &Path, src: &Path, pkg: &Path, frontend_dir: &Path) -> bool {
    if !dist.is_dir() {
        return false;
    }
    let dist_mtime = match newest_mtime(dist) {
        Some(t) => t,
        None => return false,
    };
    // 任意源文件比 dist 最新文件新即视为过期
    let checks = [
        src,
        &frontend_dir.join("index.html"),
        &frontend_dir.join("vite.config.ts"),
        &frontend_dir.join("tsconfig.json"),
        &frontend_dir.join("tsconfig.app.json"),
        &frontend_dir.join("tsconfig.node.json"),
        pkg,
    ];
    for p in checks {
        if let Some(t) = newest_mtime(p) {
            if t > dist_mtime {
                return false;
            }
        }
    }
    true
}

/// 递归查找目录下最新修改时间；文件直接返回其 mtime。
fn newest_mtime(path: &Path) -> Option<SystemTime> {
    if path.is_file() {
        return path.metadata().and_then(|m| m.modified()).ok();
    }
    if !path.is_dir() {
        return None;
    }
    let mut latest: Option<SystemTime> = None;
    walk(path, &mut latest);
    latest
}

fn walk(dir: &Path, latest: &mut Option<SystemTime>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            // 跳过 node_modules / dist / .browsers / 测试临时目录
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "node_modules" | "dist" | ".browsers" | ".pylibs" | "target"
                ) {
                    continue;
                }
            }
            walk(&p, latest);
        } else if p.is_file() {
            if let Ok(meta) = p.metadata() {
                if let Ok(m) = meta.modified() {
                    *latest = Some(match latest {
                        Some(prev) if *prev > m => *prev,
                        _ => m,
                    });
                }
            }
        }
    }
}

/// 在 frontend/ 目录执行 bun 命令；失败时 panic 并打印可读错误。
fn run_bun(cwd: &Path, args: &[&str], label: &str) {
    let (program, prefix) = which_bun();
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd);
    if let Some(pre) = prefix {
        cmd.arg(pre);
    }
    for a in args {
        cmd.arg(a);
    }
    // 编译期可见输出（仅在 cargo -vV 或出错时显示）
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => panic!("{label} 启动失败：{e}\n请确认 bun 已安装并位于 PATH 中。"),
    };
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "{label} 失败（退出码 {}）\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            output.status.code().unwrap_or(-1),
        );
    }
    println!("cargo:warning={label} 完成");
}

/// 跨平台选择 bun 命令。bun 在 Windows 是 bun.exe（直接调用即可），在其它平台为 bun。
fn which_bun() -> (&'static str, Option<&'static str>) {
    if cfg!(target_os = "windows") {
        ("bun.exe", None)
    } else {
        ("bun", None)
    }
}
