//! SpeakLab 后端。
//!
//! 这个服务是**可选**的：前端 `index.html` 单独打开就能用，不依赖它。
//! 它的职责是把「需要密钥」或「需要算力」的部分挪出浏览器：
//!
//! - 语音识别：本地跑 whisper，不把录音上传到任何第三方
//! - 练习记录：多设备同步（现在存在 localStorage 里）
//! - LLM 转发：密钥留在服务端，前端拿不到
//!
//! 设计上刻意保持薄：路由只做参数校验和序列化，业务逻辑放在
//! `domain` 里，外部依赖放在 `infra` 里，方便替换和单测。

mod asr;
mod config;
mod domain;
mod error;
mod routes;
mod state;

use std::net::SocketAddr;
use std::time::Duration;

use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        asr = config.asr.describe(),
        "SpeakLab 后端启动"
    );

    let state = AppState::new(config.clone()).await?;
    let app = build_router(state, &config);

    let addr: SocketAddr = config.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("监听 http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("已停止");
    Ok(())
}

/// 组装路由。
///
/// 中间件顺序是有意的：CORS 在最外层（预检请求不该被限流或压缩），
/// 然后是日志、压缩、请求体上限。上限放在最内层是因为它只关心
/// 真正读到 body 的那一刻。
fn build_router(state: AppState, config: &Config) -> axum::Router {
    let cors = if config.dev_mode {
        // 本地开发：前端从 file:// 或别的端口打开，直接放开
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_origin(config.allowed_origins.clone())
            .allow_methods(Any)
            .allow_headers(Any)
    };

    routes::router()
        .layer(RequestBodyLimitLayer::new(config.max_body_bytes))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("speaklab_server=info,tower_http=info,warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();
}

/// 收到 Ctrl-C 或 SIGTERM 时优雅退出，让在途请求跑完。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("无法注册 Ctrl-C 处理器");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("无法注册 SIGTERM 处理器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("收到 Ctrl-C"),
        _ = terminate => tracing::info!("收到 SIGTERM"),
    }
}

/// 给测试用的默认超时，避免外部依赖卡死整个请求。
pub const DEFAULT_UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);
