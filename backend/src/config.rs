//! 配置加载。
//!
//! 运行目录下读取 `Fnk0085-smart-car-config.jsonc`（支持 JSONC 注释）；
//! 首次启动若无该文件，则写入默认配置并尝试加载。
//! TLS 已下沉到 nginx 反代，后端仅暴露明文 HTTP（h2c 协商可选）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 默认配置文件名（运行目录下）
pub const CONFIG_FILE: &str = "Fnk0085-smart-car-config.jsonc";

/// 默认配置 JSONC 文本（首次启动写入磁盘）
const DEFAULT_CONFIG_JSONC: &str = r#"{
  // Fnk0085 智能小车后端配置
  // TLS 已下沉到 nginx 反代，后端仅监听明文 HTTP。
  "http": { "host": "0.0.0.0", "port": 8080 },
  "video": {
    "cache_seconds": 1,
    "max_devices": 32,
    "max_bytes": 4194304,
    "max_frame_bytes": 262144
  },
  "auth": {
    "token": "change-me-please",
    "frontend_password": "admin1234"
  },
  "log_level": "info"
}
"#;

/// HTTP 服务监听地址
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
}

fn default_max_bytes() -> usize {
    4 * 1024 * 1024
}

fn default_max_frame_bytes() -> usize {
    256 * 1024
}

/// 视频缓存配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoConfig {
    /// 保留时长（秒），通常为 1
    pub cache_seconds: u32,
    /// 最大设备数（防止内存溢出）
    pub max_devices: u32,
    /// 每设备视频缓存总字节上限
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    /// 单帧大小上限，超过直接丢弃
    #[serde(default = "default_max_frame_bytes")]
    pub max_frame_bytes: usize,
}

/// 鉴权配置（仅 token；TLS 由 nginx 反代处理）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 设备认证 token（HTTP Bearer）
    pub token: String,
    /// 前端访问密码（明文存配置，默认 admin1234，部署前修改）
    pub frontend_password: String,
}

/// 顶层配置（不可变，启动时一次加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub http: HttpConfig,
    pub video: VideoConfig,
    pub auth: AuthConfig,
    pub log_level: String,
}

impl Config {
    /// 加载或生成默认配置文件并解析为不可变 Config。
    /// 纯函数风格：不持有可变状态，仅 IO 副作用。
    pub fn load_or_init(path: &Path) -> Result<Self> {
        if !path.exists() {
            log::warn!("配置文件 {} 不存在，写入默认配置", path.display());
            std::fs::write(path, DEFAULT_CONFIG_JSONC)
                .with_context(|| format!("写入默认配置失败: {}", path.display()))?;
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置失败: {}", path.display()))?;
        let stripped = strip_jsonc_comments(&raw);
        let cfg: Config = serde_json::from_str(&stripped)
            .with_context(|| format!("解析配置失败: {}", path.display()))?;
        Ok(cfg)
    }

    /// HTTP 监听 socket 地址
    pub fn http_addr(&self) -> String {
        format!("{}:{}", self.http.host, self.http.port)
    }
}

/// 简单 JSONC 注释剥离器。
///
/// 仅处理行注释 `// ...` 与块注释 `/* ... */`，跳过字符串字面量内的内容。
/// 不处理嵌套块注释（JSONC 规范不需要）。
fn strip_jsonc_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    // 字符串字面量状态
    let mut in_string = false;
    let mut in_escape = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b as char);
            if in_escape {
                in_escape = false;
            } else if b == b'\\' {
                in_escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        // 不在字符串内：检查注释
        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'/' {
                // 行注释：跳到行尾
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                // 块注释：跳到 */
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2; // 跳过结尾 */
                continue;
            }
        }
        out.push(b as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_comments_keeps_strings() {
        let input = r#"{
            // 注释 1
            "a": "http://x/y", /* 块注释 */
            "b": "/* not comment */"
        }"#;
        let out = strip_jsonc_comments(input);
        assert!(out.contains("\"http://x/y\""));
        assert!(out.contains("\"/* not comment */\""));
        assert!(!out.contains("注释 1"));
        assert!(!out.contains("块注释"));
    }

    #[test]
    fn default_config_parses() {
        let stripped = strip_jsonc_comments(DEFAULT_CONFIG_JSONC);
        let cfg: Config = serde_json::from_str(&stripped).expect("默认配置可解析");
        assert_eq!(cfg.http.port, 8080);
        assert_eq!(cfg.video.cache_seconds, 1);
        assert_eq!(cfg.auth.token, "change-me-please");
        assert_eq!(cfg.auth.frontend_password, "admin1234");
    }
}
