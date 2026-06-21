use std::{env, sync::Arc};
// env 读环境变量
// Arc 线程安全的共享引用计数指针

use gateway::{router, GatewayConfig, JwtManager, PostgresUserStore, UserStore, WeChatClient};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 日志初始化
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))) // |_| ... 是闭包，类似匿名函数。这里 _ 表示“参数我不关心”
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Production should set GATEWAY_CONFIG explicitly; the default keeps local
    // development simple when running from the repository root.
    // 读取配置
    let config_path = env::var("GATEWAY_CONFIG").unwrap_or_else(|_| "gateway.toml".to_string());
    let config = GatewayConfig::from_file(config_path)?.resolve()?;

    // ? 是 Rust 里非常重要的错误传播语法：
    // - 如果成功，取出里面的值继续执行。
    // - 如果失败，直接从当前函数返回错误。

    // 连接数据库并执行 migration
    let store = PostgresUserStore::connect(&config.database_url).await?;
    // Migrations run at process start so a fresh database can boot without a
    // separate migration command in the first version of the service.
    sqlx::migrate!("../../migrations").run(store.pool()).await?;

    // 把具体 store 包成 trait object
    let store: Arc<dyn UserStore> = Arc::new(store);
    // 创建 AppState
    let state = gateway::AppState::new(
        store,
        WeChatClient::new(
            config.wechat_app_id,
            config.wechat_app_secret,
            config.wechat_api_base,
        ),
        JwtManager::new(config.jwt_secret, config.jwt_ttl_seconds),
        config.upstreams,
    );

    // 启动 HTTP 服务
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(bind = %config.bind, "gateway listening");
    axum::serve(listener, router(state)).await?;

    Ok(())
}
