use std::env;

use quote_ingester::QuoteIngesterConfig;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// quote-ingester 进程入口。
///
/// `#[tokio::main]` 会把普通的 `async fn main` 包成 Tokio 异步运行时。
/// 这个服务需要异步运行时，因为它要做 HTTP 请求、数据库请求和定时任务。
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志。默认日志级别是 info，也可以通过 RUST_LOG 环境变量覆盖。
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 默认从仓库根目录的 quote-ingester.toml 读配置。
    // 生产或临时测试时，可以用 QUOTE_INGESTER_CONFIG 指向其他配置文件。
    let config_path =
        env::var("QUOTE_INGESTER_CONFIG").unwrap_or_else(|_| "quote-ingester.toml".to_string());
    let config = QuoteIngesterConfig::from_file(config_path)?.resolve()?;

    // 先单独连接一次数据库并运行 migration。
    // migration 放在仓库根目录 migrations/，因为 gateway 和 quote-ingester 共用一个数据库。
    let store = quote_ingester::PostgresKlineStore::connect(&config.database_url).await?;
    sqlx::migrate!("../../migrations").run(store.pool()).await?;
    // migration 结束后丢掉这个临时 store。真正的 worker 会自己创建长期使用的连接池。
    drop(store);

    // 启动常驻 worker。这个函数通常不会返回，除非收到 Ctrl-C 或出现不可恢复错误。
    quote_ingester::run(config).await?;

    Ok(())
}
