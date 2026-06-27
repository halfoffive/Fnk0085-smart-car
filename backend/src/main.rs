//! 程序入口：加载配置 → 初始化 logger → 构建 AppState
//! → 启动 actix-http（明文 HTTP，可选 h2c）服务。
//!
//! 设计要点：
//! - tokio 多线程 runtime（worker = CPU 核数）。
//! - 视频/控制/事件统一走 HTTP（POST /api/device/{id}/frame、register、poll、event）。
//! - TLS 已下沉到 nginx 反代，后端只跑明文 HTTP（保留 http2 feature 以支持 h2c）。
//! - 路由：handlers::route 分发到各 API handler 与静态资源服务。
//! - 优雅退出：Ctrl-C 时停止服务。

mod config;
mod device;
mod handlers;
mod protocol;
mod state;
mod static_files;
mod video_cache;

use std::path::PathBuf;
use std::sync::Arc;

use actix_http::{HttpService, Request};
use actix_server::Server;
use anyhow::Result;

use crate::config::Config;
use crate::device::DeviceRegistry;
use crate::state::AppState;

#[actix_rt::main]
async fn main() -> Result<()> {
    // 1. 加载或生成配置
    let config_path = PathBuf::from(config::CONFIG_FILE);
    let cfg = Config::load_or_init(&config_path)?;

    // 2. 初始化 logger
    let _ =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cfg.log_level))
            .try_init();

    log::info!("Fnk0085 后端启动中：HTTP {}", cfg.http_addr());

    // 3. 期望 token（用于设备 Bearer 校验）
    let expected_token = std::sync::Arc::new(cfg.auth.token.clone());

    // 4. 设备注册表（每个 DeviceEntry 内含指令队列，供 HTTP 长轮询消费）
    let registry = std::sync::Arc::new(DeviceRegistry::new(
        cfg.video.max_devices,
        // 视频缓存参数：广播容量 16，最近帧保留 8（约 1s @ 8fps）
        16,
        8,
    ));

    // 5. 共享状态
    let state = AppState {
        registry: registry.clone(),
        expected_token: expected_token.clone(),
        log_level: Arc::new(cfg.log_level.clone()),
        server_addr: cfg.http_addr(),
    };

    // 6. 启动 actix-http 明文 HTTP 服务（h2c 协商由 http2 feature 支持）
    let http_addr = cfg.http_addr();
    let state_for_http = state.clone();

    let server = Server::build()
        .bind("http", &http_addr, move || {
            let state = state_for_http.clone();
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
                // 明文 TCP，并启用 h2c（HTTP/2 over plaintext）协商，供 nginx 反代或客户端直连使用
                .tcp_auto_h2c()
        })?
        .workers(num_cpus()) // worker 数 = CPU 核数
        .run();

    log::info!("HTTP 服务已就绪：http://{http_addr}");
    server.await?;
    Ok(())
}

/// 当前 CPU 核数
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
