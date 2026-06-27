//! 程序入口：加载配置 → 初始化 logger → 自签 TLS 证书 → 构建 AppState
//! → 启动 UDP 监听任务 → 启动 actix-http（HTTPS via rustls）服务。
//!
//! 设计要点：
//! - tokio 多线程 runtime（worker = CPU 核数）。
//! - UDP 监听任务独立 spawn，不阻塞 HTTP 服务。
//! - HTTP/HTTPS：actix-server + HttpService，rustls 0.23 完成 TLS。
//! - 路由：handlers::route 分发到各 API handler 与静态资源服务。
//! - 优雅退出：Ctrl-C 时停止服务。

mod config;
mod crypto;
mod device;
mod handlers;
mod protocol;
mod state;
mod static_files;
mod udp_listener;
mod video_cache;

use std::path::PathBuf;
use std::sync::Arc;

use actix_http::{HttpService, Request};
use actix_server::Server;
use actix_service::ServiceFactoryExt;
use anyhow::{Context, Result};
use rustls::ServerConfig;

use crate::config::Config;
use crate::crypto::derive_key;
use crate::device::{CommandSink, DeviceRegistry, OutboundCommand};
use crate::state::AppState;

#[actix_rt::main]
async fn main() -> Result<()> {
    // 1. 加载或生成配置
    let config_path = PathBuf::from(config::CONFIG_FILE);
    let cfg = Config::load_or_init(&config_path)?;

    // 2. 初始化 logger
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(&cfg.log_level),
    )
    .try_init();

    log::info!(
        "Fnk0085 后端启动中：HTTP {} UDP {}",
        cfg.http_addr(),
        cfg.udp_addr()
    );

    // 3. 加载或自签 TLS 证书
    let (cert_pem, key_pem, ca_pem) =
        config::load_or_generate_tls_materials(&cfg.auth)?;

    // 4. 派生 AEAD 密钥（不可变句柄）
    let key = Arc::new(derive_key(&cfg.auth.token));
    let expected_token = Arc::new(cfg.auth.token.clone());

    // 5. 设备注册表 + 指令通道
    let registry = Arc::new(DeviceRegistry::new(
        cfg.video.max_devices,
        // 视频缓存参数：广播容量 16，最近帧保留 8（约 1s @ 8fps）
        16,
        8,
    ));
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<OutboundCommand>(64);
    let cmd_sink = CommandSink::new(cmd_tx);

    // 6. 共享状态
    let state = AppState {
        registry: registry.clone(),
        key: key.clone(),
        expected_token: expected_token.clone(),
        cmd_sink,
        video_cache_capacity: 16,
        video_max_recent: 8,
    };

    // 7. 启动 UDP 监听任务
    let udp_addr = cfg.udp_addr();
    let udp_state = state.clone();
    actix_rt::spawn(async move {
        if let Err(e) = udp_listener::run(
            udp_addr,
            udp_state.expected_token,
            udp_state.key,
            udp_state.registry,
            cmd_rx,
        )
        .await
        {
            log::error!("UDP 监听退出: {e}");
        }
    });

    // 8. 构建 rustls ServerConfig
    let server_config = build_rustls_server_config(&cert_pem, &key_pem, ca_pem.as_deref())
        .context("构建 rustls::ServerConfig 失败")?;

    // 9. 启动 actix-http + rustls HTTPS 服务
    let http_addr = cfg.http_addr();
    let state_for_http = state.clone();

    let server = Server::build()
        .bind("http-tls", &http_addr, move || {
            let state = state_for_http.clone();
            let config = server_config.clone();
            HttpService::build()
                .finish(move |req: Request| {
                    let state = state.clone();
                    async move {
                        // route 返回 Result<AppResponse, Infallible>，永远不会失败
                        match handlers::route(req, state).await {
                            Ok(resp) => Ok::<_, std::convert::Infallible>(resp),
                            Err(e) => match e {},
                        }
                    }
                })
                .rustls_0_23(config)
                .map_err(|e| {
                    log::error!("HTTP 服务错误: {e:?}");
                    std::io::Error::new(std::io::ErrorKind::Other, format!("{e:?}"))
                })
        })?
        .workers(num_cpus()) // worker 数 = CPU 核数
        .run();

    log::info!("HTTPS 服务已就绪：https://{http_addr}");
    server.await.context("actix-server 运行错误")?;
    Ok(())
}

/// 构建 rustls 0.23 ServerConfig。
///
/// - `cert_pem`：服务端证书链（PEM，多证书按顺序）。
/// - `key_pem`：私钥（PEM，支持 PKCS8 / PKCS1 / SEC1）。
/// - `client_ca_pem`：可选的客户端 CA（mTLS 启用时强制校验）。
fn build_rustls_server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    client_ca_pem: Option<&[u8]>,
) -> Result<ServerConfig> {
    use rustls::pki_types::CertificateDer;
    use rustls::server::WebPkiClientVerifier;

    // 解析证书链（&[u8] 实现 BufRead，需可变绑定以供 reader 推进游标）
    let mut cert_reader = cert_pem;
    let cert_chain: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("解析 TLS 证书 PEM 失败")?;

    if cert_chain.is_empty() {
        anyhow::bail!("TLS 证书链为空");
    }

    // 解析私钥（rustls_pemfile::private_key 已返回 PrivateKeyDer<'static>）
    let mut key_reader = key_pem;
    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("解析 TLS 私钥 PEM 失败")?
        .ok_or_else(|| anyhow::anyhow!("未找到 TLS 私钥"))?;

    // 构造 ServerConfig：rustls 0.23 API 要求先选择 verifier（WantsVerifier → WantsServerCert），
    // 再 with_single_cert 完成构造。
    let mut config = match client_ca_pem {
        Some(ca) => {
            // mTLS：加载 CA 并要求客户端证书
            let mut ca_reader = ca;
            let ca_certs = rustls_pemfile::certs(&mut ca_reader)
                .collect::<Result<Vec<_>, _>>()
                .context("解析 client CA PEM 失败")?;
            if ca_certs.is_empty() {
                anyhow::bail!("client CA 配置但 PEM 为空");
            }
            let mut root_store = rustls::RootCertStore::empty();
            for c in ca_certs {
                root_store
                    .add(c)
                    .context("添加 client CA 到 root store 失败")?;
            }
            let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
                .build()
                .context("构造 client verifier 失败")?;
            ServerConfig::builder()
                .with_client_cert_verifier(verifier)
                .with_single_cert(cert_chain, key)
                .context("构造 rustls::ServerConfig 失败")?
        }
        None => ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)
            .context("构造 rustls::ServerConfig 失败")?,
    };

    // ALPN：HTTP/1.1 与 HTTP/2
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(config)
}

/// 当前 CPU 核数
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
