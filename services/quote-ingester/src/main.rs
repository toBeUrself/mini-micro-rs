use std::env;

use quote_ingester::QuoteIngesterConfig;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config_path =
        env::var("QUOTE_INGESTER_CONFIG").unwrap_or_else(|_| "quote-ingester.toml".to_string());
    let config = QuoteIngesterConfig::from_file(config_path)?.resolve()?;

    let store = quote_ingester::PostgresKlineStore::connect(&config.database_url).await?;
    sqlx::migrate!("../../migrations").run(store.pool()).await?;
    drop(store);

    quote_ingester::run(config).await?;

    Ok(())
}
