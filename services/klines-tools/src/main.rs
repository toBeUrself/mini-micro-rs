//! klines-tools 分析服务入口。
//!
//! 启动 HTTP 服务，提供以下 API：
//! - GET /api/v1/tools/analysis/market-state
//! - GET /api/v1/tools/analysis/grid-plan
//! - GET /api/v1/tools/analysis/signals
//! - GET /api/v1/tools/analysis/multi-timeframe-state
//! - GET /api/v1/tools/analysis/marks

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

/// 共享应用状态。
struct AppState {
    analyzer: Analyzer,
    default_source: String,
}

/// 市场状态查询参数。
#[derive(Debug, Deserialize)]
struct MarketStateQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

/// 网格计划查询参数。
#[derive(Debug, Deserialize)]
struct GridPlanQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

/// 信号查询参数。
#[derive(Debug, Deserialize)]
struct SignalsQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

/// 多周期查询参数。
#[derive(Debug, Deserialize)]
struct MultiTfQuery {
    source: Option<String>,
    symbol: String,
}

/// Marks 查询参数（TradingView getMarks）。
#[derive(Debug, Deserialize)]
struct MarksQuery {
    source: Option<String>,
    symbol: String,
    interval: String,
    time: Option<i64>,
}

/// 错误响应。
#[derive(Debug, serde::Serialize)]
struct ApiErrorResponse {
    error: String,
    code: String,
}

impl ApiErrorResponse {
    fn new(code: &str, msg: &str) -> Self {
        Self {
            error: msg.to_string(),
            code: code.to_string(),
        }
    }
}

/// 从查询参数中解析 source，如果未指定则使用默认值。
fn resolve_source(source: Option<String>, default_source: &str) -> String {
    source.unwrap_or_else(|| default_source.to_string())
}

/// 将错误信息映射为 HTTP 状态码和结构化错误响应。
fn map_analysis_error(e: String) -> (axum::http::StatusCode, Json<ApiErrorResponse>) {
    let (code, status) = if e.contains("no klines data") || e.contains("warmup") {
        ("INSUFFICIENT_KLINES", axum::http::StatusCode::BAD_REQUEST)
    } else if e.contains("data quality") || e.contains("invalid") {
        ("DATA_QUALITY_LOW", axum::http::StatusCode::BAD_REQUEST)
    } else if e.contains("indicator") || e.contains("warmup") {
        ("INDICATOR_UNAVAILABLE", axum::http::StatusCode::SERVICE_UNAVAILABLE)
    } else if e.contains("config") {
        ("CONFIG_NOT_FOUND", axum::http::StatusCode::INTERNAL_SERVER_ERROR)
    } else if e.contains("exchange") || e.contains("constraint") {
        ("EXCHANGE_CONSTRAINT_FAILED", axum::http::StatusCode::BAD_REQUEST)
    } else if e.contains("disabled") {
        ("FEATURE_DISABLED", axum::http::StatusCode::BAD_REQUEST)
    } else {
        ("INTERNAL_ERROR", axum::http::StatusCode::INTERNAL_SERVER_ERROR)
    };

    (status, Json(ApiErrorResponse::new(code, &e)))
}

/// GET /api/v1/tools/analysis/market-state
async fn get_market_state(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MarketStateQuery>,
) -> Result<Json<AnalysisOutput>, (axum::http::StatusCode, Json<ApiErrorResponse>)> {
    let source = resolve_source(params.source, &state.default_source);
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(map_analysis_error)?;
    Ok(Json(output))
}

/// GET /api/v1/tools/analysis/grid-plan
async fn get_grid_plan(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GridPlanQuery>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<ApiErrorResponse>)> {
    let source = resolve_source(params.source, &state.default_source);
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(map_analysis_error)?;
    Ok(Json(serde_json::json!({
        "symbol": output.symbol,
        "interval": output.interval,
        "time": output.time,
        "state": output.state,
        "state_phase": output.state_phase,
        "grid_plan": output.grid_plan,
        "confidence": output.confidence_breakdown.final_confidence,
    })))
}

/// GET /api/v1/tools/analysis/signals
async fn get_signals(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SignalsQuery>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<ApiErrorResponse>)> {
    let source = resolve_source(params.source, &state.default_source);
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(map_analysis_error)?;
    Ok(Json(serde_json::json!({
        "symbol": output.symbol,
        "interval": output.interval,
        "time": output.time,
        "state": output.state,
        "state_phase": output.state_phase,
        "signals": output.signals,
    })))
}

/// GET /api/v1/tools/analysis/multi-timeframe-state
async fn get_multi_timeframe_state(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MultiTfQuery>,
) -> Result<Json<MultiTfAnalysisOutput>, (axum::http::StatusCode, Json<ApiErrorResponse>)> {
    let source = resolve_source(params.source, &state.default_source);
    let output = state
        .analyzer
        .analyze_multi_tf(&source, &params.symbol)
        .await
        .map_err(map_analysis_error)?;
    Ok(Json(output))
}

/// GET /api/v1/tools/analysis/marks
/// TradingView getMarks / getTimescaleMarks 兼容端点。
async fn get_marks(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MarksQuery>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<ApiErrorResponse>)> {
    let source = resolve_source(params.source, &state.default_source);
    let output = state
        .analyzer
        .analyze_single(&source, &params.symbol, &params.interval, params.time)
        .await
        .map_err(map_analysis_error)?;

    // 将 signals 转换为 TradingView marks 格式
    let marks: Vec<serde_json::Value> = output
        .signals
        .iter()
        .map(|s| {
            let (label, color) = match s.signal_type {
                SignalType::UpBreakWarning => ("↑W", "orange"),
                SignalType::DownBreakWarning => ("↓W", "red"),
                SignalType::PauseGrid => ("⏸", "red"),
                SignalType::ResumeGrid => ("▶", "green"),
                SignalType::RiskReduce => ("⚠", "darkred"),
                SignalType::MoveGridUp => ("↑", "blue"),
                SignalType::MoveGridDown => ("↓", "blue"),
                SignalType::GridBuyWatch => ("B", "green"),
                SignalType::GridSellWatch => ("S", "red"),
            };
            serde_json::json!({
                "id": s.time,
                "time": s.time,
                "color": color,
                "text": s.text,
                "label": label,
                "labelFontColor": "white",
                "minSize": 15,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "symbol": output.symbol,
        "interval": output.interval,
        "marks": marks,
    })))
}

/// GET /health
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
            tracing::warn!("Cannot read {config_path}: {e}, using built-in defaults");
            // 使用合理的默认配置，不会 panic
            let default_toml = r#"
bind = "127.0.0.1:8081"
app_api_base_url = "http://127.0.0.1:8080"
"#;
            KlinesToolsConfig::parse(default_toml).expect("built-in default config should be valid")
        }
    };

    let bind = config.bind.clone();
    let timeout = Duration::from_secs(config.http_timeout_secs);
    let default_source = config.default_source.clone();

    let reader =
        KlineReader::new(&config.app_api_base_url, timeout).expect("failed to create KlineReader");

    let analyzer = Analyzer::new(config, reader);

    let app_state = Arc::new(AppState { analyzer, default_source });

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/tools/analysis/market-state", get(get_market_state))
        .route("/api/v1/tools/analysis/grid-plan", get(get_grid_plan))
        .route("/api/v1/tools/analysis/signals", get(get_signals))
        .route(
            "/api/v1/tools/analysis/multi-timeframe-state",
            get(get_multi_timeframe_state),
        )
        .route("/api/v1/tools/analysis/marks", get(get_marks))
        .with_state(app_state);

    let addr: SocketAddr = bind.parse().expect("invalid bind address");
    tracing::info!("klines-tools listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind failed");
    axum::serve(listener, app).await.expect("server error");
}
