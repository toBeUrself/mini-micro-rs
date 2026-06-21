use std::env;

use app_api::{router, AppApiConfig, AppState, PostgresAppStore};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// app-api 的进程入口。
///
/// `#[tokio::main]` 会启动 Tokio 异步运行时。
/// 因为 HTTP 服务、数据库访问都是异步 IO，所以这里需要 Tokio。
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志。可以通过 RUST_LOG=debug 调整日志级别。
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 默认从仓库根目录下的 app-api.toml 读取配置。
    // 也可以通过 APP_API_CONFIG 指定其他配置文件路径。
    let config_path = env::var("APP_API_CONFIG").unwrap_or_else(|_| "app-api.toml".to_string());
    let config = AppApiConfig::from_file(config_path)?.resolve()?;

    // 连接数据库。这个服务目前主要读数据库，但本地开发时也顺手执行 migration，
    // 这样只启动 app-api 时也能确保表结构存在。
    let store = PostgresAppStore::connect(&config.database_url).await?;
    sqlx::migrate!("../../migrations").run(store.pool()).await?;

    let state = AppState::new(store);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;

    tracing::info!(bind = %config.bind, "app-api listening");
    axum::serve(listener, router(state)).await?;

    Ok(())
}
