use axum::{http::StatusCode, routing::get, Router};

use crate::{market, store::PostgresAppStore};

/// app-api 全局共享状态。
///
/// axum 会把这个对象 clone 后传给每个 handler。
/// 因为里面的 `PostgresAppStore` 只是包了一层连接池，所以 clone 成本很低。
#[derive(Clone)]
pub struct AppState {
    /// 数据库访问对象。
    pub(crate) store: PostgresAppStore,
}

impl AppState {
    /// 创建应用状态。
    pub fn new(store: PostgresAppStore) -> Self {
        Self { store }
    }
}

/// 创建 axum Router。
///
/// 这里集中注册 app-api 自己提供的所有接口。
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/public/market/klines", get(market::get_klines))
        .route("/api/v1/public/market/symbols", get(market::list_symbols))
        .with_state(state)
}

/// 健康检查接口。
async fn healthz() -> StatusCode {
    StatusCode::OK
}
