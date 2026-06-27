//! 配置加载与自签证书生成。
//!
//! 运行目录下读取 `Fnk0085-smart-car-config.jsonc`（支持 JSONC 注释）；
//! 首次启动若无该文件，则写入默认配置并尝试加载。
//! 若配置中引用的 TLS 证书不存在，则用 rcgen 自签生成占位证书（dev 友好）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 默认配置文件名（运行目录下）
pub const CONFIG_FILE: &str = "Fnk0085-smart-car-config.jsonc";

/// 默认配置 JSONC 文本（首次启动写入磁盘）
const DEFAULT_CONFIG_JSONC: &str = r#"{
  // Fnk0085 智能小车后端配置
  "http": { "host": "0.0.0.0", "port": 8080 },
  "udp": { "host": "0.0.0.0", "port": 7000 },
  "video": { "cache_seconds": 1, "max_devices": 32 },
  "auth": {
    "token": "change-me-please",
    "tls_cert": "./certs/server.crt",
    "tls_key": "./certs/server.key",
    "client_ca": "./certs/ca.crt"
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

/// UDP 服务监听地址（接收设备分包）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpConfig {
    pub host: String,
    pub port: u16,
}

/// 视频缓存配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoConfig {
    /// 保留时长（秒），通常为 1
    pub cache_seconds: u32,
    /// 最大设备数（防止内存溢出）
    pub max_devices: u32,
}

/// 鉴权与证书配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 设备认证 token（用于派生 AES-128-GCM 密钥）
    pub token: String,
    /// 服务端证书
    pub tls_cert: String,
    /// 服务端私钥
    pub tls_key: String,
    /// 客户端 CA（mTLS 校验，可选）
    pub client_ca: Option<String>,
}

/// 顶层配置（不可变，启动时一次加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub http: HttpConfig,
    pub udp: UdpConfig,
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

    /// UDP 监听 socket 地址
    pub fn udp_addr(&self) -> String {
        format!("{}:{}", self.udp.host, self.udp.port)
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

/// 加载或自签生成 TLS 服务端证书与客户端 CA（若配置启用 mTLS）。
///
/// 返回 (cert_chain_pem, key_pem, client_ca_pem_opt)。
/// 若文件已存在则直接读取；否则用 rcgen 自签生成并落盘。
pub fn load_or_generate_tls_materials(cfg: &AuthConfig) -> Result<(Vec<u8>, Vec<u8>, Option<Vec<u8>>)> {
    let cert_path = Path::new(&cfg.tls_cert);
    let key_path = Path::new(&cfg.tls_key);
    if cert_path.exists() && key_path.exists() {
        let cert = std::fs::read(cert_path).context("读取 tls_cert 失败")?;
        let key = std::fs::read(key_path).context("读取 tls_key 失败")?;
        let ca = match &cfg.client_ca {
            Some(p) if Path::new(p).exists() => Some(std::fs::read(p).context("读取 client_ca 失败")?),
            _ => None,
        };
        return Ok((cert, key, ca));
    }
    log::warn!("TLS 证书缺失，使用 rcgen 自签生成占位证书（仅适用 dev/测试）");
    let (cert, key, ca) = gen_self_signed_pair(cfg)?;
    // 落盘便于下次复用
    ensure_parent(cert_path)?;
    ensure_parent(key_path)?;
    std::fs::write(cert_path, &cert).context("写入 tls_cert 失败")?;
    std::fs::write(key_path, &key).context("写入 tls_key 失败")?;
    if let Some(ca_ref) = &cfg.client_ca {
        ensure_parent(Path::new(ca_ref))?;
        std::fs::write(ca_ref, &ca).context("写入 client_ca 失败")?;
    }
    Ok((cert, key, if cfg.client_ca.is_some() { Some(ca) } else { None }))
}

fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).context("创建证书目录失败")?;
        }
    }
    Ok(())
}

/// 使用 rcgen 自签一对服务端证书 + 客户端 CA（签名关系）。
/// 返回 (server_cert_pem, server_key_pem, client_ca_pem)。
/// rcgen 0.13 API：CertificateParams::self_signed / signed_by 直接生成 Certificate。
fn gen_self_signed_pair(_cfg: &AuthConfig) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, BasicConstraints, KeyPair};

    // 1. CA 参数（用于客户端 mTLS 校验）
    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| anyhow::anyhow!("CA params: {e}"))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::OrganizationName, "Fnk0085");
    ca_dn.push(DnType::CommonName, "Fnk0085 Dev CA");
    ca_params.distinguished_name = ca_dn;
    let ca_key = KeyPair::generate().map_err(|e| anyhow::anyhow!("CA key: {e}"))?;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| anyhow::anyhow!("CA self_signed: {e}"))?;

    // 2. 服务端证书由 CA 签名，SAN 包含 localhost + 内网常用名
    let mut server_params = CertificateParams::new(vec!["localhost".to_string()])
        .map_err(|e| anyhow::anyhow!("server params: {e}"))?;
    let mut server_dn = DistinguishedName::new();
    server_dn.push(DnType::OrganizationName, "Fnk0085");
    server_dn.push(DnType::CommonName, "Fnk0085 Dev Server");
    server_params.distinguished_name = server_dn;
    let server_key = KeyPair::generate().map_err(|e| anyhow::anyhow!("server key: {e}"))?;
    let signed_server = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .map_err(|e| anyhow::anyhow!("server signed_by: {e}"))?;

    let cert_pem = signed_server.pem().into_bytes();
    let key_pem = server_key.serialize_pem().into_bytes();
    let ca_pem = ca_cert.pem().into_bytes();
    Ok((cert_pem, key_pem, ca_pem))
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
        assert_eq!(cfg.udp.port, 7000);
        assert_eq!(cfg.video.cache_seconds, 1);
        assert_eq!(cfg.auth.token, "change-me-please");
    }
}
