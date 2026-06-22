//! klines-tools 分析服务入口。
//!
//! 启动 HTTP 服务，提供只读分析 API。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::Json,
    routing::get,
    Router,
};
use serde::Deserialize;

use klines_tools::{
    analyzer::Analyzer, config::KlinesToolsConfig, kline_reader::KlineReader, models::*,
};

struct AppState {
    analyzer: Analyzer,
}

#[derive(Debug, Deserialize)]
struct MarketStateQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GridPlanQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SignalsQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MultiTfQuery {
    source: Option<String>,
    symbol: String,
}

async fn get_market_state(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MarketStateQuery>,
) -> Result<Json<AnalysisOutput>, axum::http::StatusCode> {
    let source = params
        .source
        .unwrap_or_else(|| state.analyzer.config.default_source.clone());
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(|e| {
            tracing::error!("market-state failed: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(output))
}

async fn get_grid_plan(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GridPlanQuery>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let source = params
        .source
        .unwrap_or_else(|| state.analyzer.config.default_source.clone());
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(|e| {
            tracing::error!("grid-plan failed: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(serde_json::json!({
        "symbol": output.symbol,
        "interval": output.interval,
        "time": output.time,
        "state": output.state,
        "state_phase": output.state_phase,
        "risk_override": output.risk_override,
        "risk_decision": output.risk_decision,
        "grid_plan": output.grid_plan,
        "confidence": output.confidence_breakdown.final_confidence,
    })))
}

async fn get_signals(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SignalsQuery>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let source = params
        .source
        .unwrap_or_else(|| state.analyzer.config.default_source.clone());
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(|e| {
            tracing::error!("signals failed: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(serde_json::json!({
        "symbol": output.symbol,
        "interval": output.interval,
        "time": output.time,
        "state": output.state,
        "state_phase": output.state_phase,
        "signals": output.signals,
    })))
}

async fn get_multi_timeframe_state(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MultiTfQuery>,
) -> Result<Json<MultiTfAnalysisOutput>, axum::http::StatusCode> {
    let source = params
        .source
        .unwrap_or_else(|| state.analyzer.config.default_source.clone());
    let output = state
        .analyzer
        .analyze_multi_tf(&source, &params.symbol)
        .await
        .map_err(|e| {
            tracing::error!("multi-tf failed: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(output))
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "klines_tools=info".into()),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "klines-tools.toml".to_string());

    let config = match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            tracing::info!("Loading config from {config_path}");
            KlinesToolsConfig::parse(&content).expect("invalid config TOML")
        }
        Err(e) => {
            tracing::warn!("Cannot read {config_path}: {e}, using defaults");
            toml::from_str("").expect("could not create default config")
        }
    };

    let bind = config.bind.clone();
    let timeout = Duration::from_secs(config.http_timeout_secs);
    let reader =
        KlineReader::new(&config.app_api_base_url, timeout).expect("failed to create KlineReader");
    let analyzer = Analyzer::new(config, reader);
    let app_state = Arc::new(AppState { analyzer });

    // 同时提供 spec 约定路径和历史 /tools 路径，方便兼容。
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/analysis/market-state", get(get_market_state))
        .route("/api/v1/analysis/grid-plan", get(get_grid_plan))
        .route("/api/v1/analysis/signals", get(get_signals))
        .route("/api/v1/analysis/multi-timeframe-state", get(get_multi_timeframe_state))
        .route("/api/v1/tools/analysis/market-state", get(get_market_state))
        .route("/api/v1/tools/analysis/grid-plan", get(get_grid_plan))
        .route("/api/v1/tools/analysis/signals", get(get_signals))
        .route("/api/v1/tools/analysis/multi-timeframe-state", get(get_multi_timeframe_state))
        .with_state(app_state);

    let addr: SocketAddr = bind.parse().expect("invalid bind address");
    tracing::info!("klines-tools listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind failed");
    axum::serve(listener, app).await.expect("server error");
}
