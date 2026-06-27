//! 编译期检查 frontend/dist 是否存在，缺失则给出可读构建错误。
//!
//! 由于 `include_dir!` 在路径不存在时无法直接给出友好提示，
//! 这里通过 build.rs 提前断言。

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("frontend")
        .join("dist");
    if !dist.exists() {
        panic!(
            "frontend/dist 不存在：{}\n请先在 frontend/ 目录执行 `npm run build` 生成 dist/。",
            dist.display()
        );
    }
    // 顶层目录变更时重新触发 include_dir 重编译
    println!("cargo:rerun-if-changed=../frontend/dist");
}
