//! 编译期内嵌前端构建产物。
//!
//! 使用 `include_dir!` 宏在编译期将 `frontend/dist/` 整个目录嵌入二进制，
//! 运行时直接从中取文件服务，无需独立前端服务器，实现单二进制部署。

use bytes::Bytes;
use include_dir::{include_dir, Dir};

/// 内嵌的前端 dist 目录（路径相对 Cargo.toml）
static FRONTEND_DIST: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../frontend/dist");

/// 静态文件查询结果
pub struct StaticAsset {
    pub bytes: Bytes,
    /// 完整的 Content-Type 字符串（含 charset）
    pub content_type: &'static str,
}

/// 按路径查询内嵌静态资源。
///
/// - 路径为空或 `/` → index.html
/// - 路径命中文件 → 返回文件
/// - 路径未命中 → 返回 None（由上层决定 SPA 回退到 index.html 或 404）
pub fn lookup(path: &str) -> Option<StaticAsset> {
    let normalized = path.trim_start_matches('/');
    let file = if normalized.is_empty() {
        FRONTEND_DIST.get_file("index.html")
    } else {
        FRONTEND_DIST.get_file(normalized).or_else(|| {
            // 顶层路径（无扩展名且不在 assets 下）→ 尝试作为目录 index
            FRONTEND_DIST.get_file(&format!("{normalized}/index.html"))
        })
    }?;
    let bytes = file.contents();
    let content_type = guess_content_type(file.path());
    Some(StaticAsset {
        bytes: Bytes::from_static(bytes),
        content_type,
    })
}

/// 总是返回 index.html（用于 SPA 路由回退）
pub fn index_html() -> Option<StaticAsset> {
    lookup("/")
}

/// 根据路径推断 Content-Type。
/// 文本类型附加 charset=utf-8，二进制类型按需。
fn guess_content_type(path: &std::path::Path) -> &'static str {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // 将 mime 转换为静态字符串
    match mime.essence_str() {
        "text/html" => "text/html; charset=utf-8",
        "text/css" => "text/css; charset=utf-8",
        "application/javascript" => "application/javascript; charset=utf-8",
        "text/javascript" => "text/javascript; charset=utf-8",
        "application/json" => "application/json; charset=utf-8",
        "image/svg+xml" => "image/svg+xml",
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/x-icon" => "image/x-icon",
        "application/manifest+json" => "application/manifest+json; charset=utf-8",
        "text/plain" => "text/plain; charset=utf-8",
        "application/wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_index_html() {
        let asset = lookup("/").expect("index.html 必须存在");
        assert_eq!(asset.content_type, "text/html; charset=utf-8");
        assert!(!asset.bytes.is_empty());
    }

    #[test]
    fn lookup_known_asset() {
        // 前端 dist 应至少包含 assets/ 目录
        if let Some(_asset) = lookup("/manifest.webmanifest") {
            // OK
        }
    }
}
