use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    error::ApiError,
    store::{KlineMarketSummary, KlineQuery, KlineRow},
};

const DEFAULT_SOURCE: &str = "binance";
const DEFAULT_LIMIT: i64 = 500;
const MAX_LIMIT: i64 = 1000;

/// `/api/v1/market/klines` 支持的 query string。
///
/// axum 会把类似下面的 URL 参数反序列化成这个结构体：
/// `/api/v1/market/klines?symbol=BTCUSDT&interval=1m&limit=100`
#[derive(Debug, Deserialize)]
pub struct KlineQueryParams {
    /// 数据来源，可选。不传时默认 `binance`。
    pub source: Option<String>,
    /// 交易对，必填，例如 `BTCUSDT`。
    pub symbol: String,
    /// 周期，必填，例如 `1m`、`5m`、`30m`。
    pub interval: String,
    /// 开始时间，毫秒时间戳。支持 `startTime`，也兼容 `start_time`。
    #[serde(rename = "startTime", alias = "start_time")]
    pub start_time: Option<i64>,
    /// 结束时间，毫秒时间戳。支持 `endTime`，也兼容 `end_time`。
    #[serde(rename = "endTime", alias = "end_time")]
    pub end_time: Option<i64>,
    /// 返回数量，可选，默认 500，最大 1000。
    pub limit: Option<i64>,
}

/// K 线列表响应。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KlinesResponse {
    /// 数据来源。
    pub source: String,
    /// 交易对。
    pub symbol: String,
    /// 周期。
    pub interval: String,
    /// K 线数组。
    pub items: Vec<KlineResponse>,
}

/// 单根 K 线响应。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KlineResponse {
    /// 开盘时间，毫秒时间戳。
    pub open_time: i64,
    /// 开盘价。用字符串返回，避免 JavaScript number 精度问题。
    #[serde(rename = "open")]
    pub open_price: String,
    /// 最高价。
    #[serde(rename = "high")]
    pub high_price: String,
    /// 最低价。
    #[serde(rename = "low")]
    pub low_price: String,
    /// 收盘价。
    #[serde(rename = "close")]
    pub close_price: String,
    /// 基础币成交量。
    pub base_volume: String,
    /// 计价币成交额。
    pub quote_volume: String,
    /// 聚合时使用了多少根源 K 线。
    pub source_count: i32,
    /// 是否完整。
    pub is_complete: bool,
}

/// 市场数据概览响应。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSymbolsResponse {
    /// 当前数据库里已有的 source/symbol/interval 组合。
    pub items: Vec<MarketSymbolResponse>,
}

/// 单个 source/symbol/interval 的数据概览。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSymbolResponse {
    /// 数据来源。
    pub source: String,
    /// 交易对。
    pub symbol: String,
    /// 周期。
    pub interval: String,
    /// 最早数据时间，毫秒时间戳。
    pub start_time: i64,
    /// 最新数据时间，毫秒时间戳。
    pub end_time: i64,
    /// 总行数。
    pub row_count: i64,
}

/// 查询 K 线列表的 handler。
pub async fn get_klines(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<KlineQueryParams>,
) -> Result<Json<KlinesResponse>, ApiError> {
    let query = params.try_into_query()?;
    let rows = state.store.query_klines(query.clone()).await?;

    Ok(Json(KlinesResponse {
        source: query.source,
        symbol: query.symbol,
        interval: query.interval,
        items: rows.into_iter().map(KlineResponse::from).collect(),
    }))
}

/// 查询数据库里有哪些行情数据的 handler。
pub async fn list_symbols(
    State(state): State<AppState>,
) -> Result<Json<MarketSymbolsResponse>, ApiError> {
    let rows = state.store.list_kline_markets().await?;
    Ok(Json(MarketSymbolsResponse {
        items: rows.into_iter().map(MarketSymbolResponse::from).collect(),
    }))
}

impl KlineQueryParams {
    /// 把 HTTP query 参数转换成数据库查询参数。
    fn try_into_query(self) -> Result<KlineQuery, ApiError> {
        let source = self
            .source
            .unwrap_or_else(|| DEFAULT_SOURCE.to_string())
            .trim()
            .to_ascii_lowercase();
        let symbol = self.symbol.trim().to_ascii_uppercase();
        let interval = self.interval.trim().to_ascii_lowercase();
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);

        if source.is_empty() {
            return Err(ApiError::BadRequest("source is required".to_string()));
        }
        if symbol.is_empty() {
            return Err(ApiError::BadRequest("symbol is required".to_string()));
        }
        if interval.is_empty() {
            return Err(ApiError::BadRequest("interval is required".to_string()));
        }
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(ApiError::BadRequest(format!(
                "limit must be between 1 and {MAX_LIMIT}"
            )));
        }

        let start_time = self.start_time.map(timestamp_millis_to_utc).transpose()?;
        let end_time = self.end_time.map(timestamp_millis_to_utc).transpose()?;

        if let (Some(start_time), Some(end_time)) = (start_time, end_time) {
            if start_time >= end_time {
                return Err(ApiError::BadRequest(
                    "startTime must be earlier than endTime".to_string(),
                ));
            }
        }

        Ok(KlineQuery {
            source,
            symbol,
            interval,
            start_time,
            end_time,
            limit,
        })
    }
}

impl From<KlineRow> for KlineResponse {
    fn from(row: KlineRow) -> Self {
        Self {
            open_time: row.open_time.timestamp_millis(),
            open_price: row.open_price.to_string(),
            high_price: row.high_price.to_string(),
            low_price: row.low_price.to_string(),
            close_price: row.close_price.to_string(),
            base_volume: row.base_volume.to_string(),
            quote_volume: row.quote_volume.to_string(),
            source_count: row.source_count,
            is_complete: row.is_complete,
        }
    }
}

impl From<KlineMarketSummary> for MarketSymbolResponse {
    fn from(row: KlineMarketSummary) -> Self {
        Self {
            source: row.source,
            symbol: row.symbol,
            interval: row.interval,
            start_time: row.start_time.timestamp_millis(),
            end_time: row.end_time.timestamp_millis(),
            row_count: row.row_count,
        }
    }
}

/// 把毫秒时间戳转换成 UTC 时间。
fn timestamp_millis_to_utc(value: i64) -> Result<DateTime<Utc>, ApiError> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid timestamp millis: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_params_use_defaults_and_normalize_text() {
        let query = KlineQueryParams {
            source: None,
            symbol: " btcusdt ".to_string(),
            interval: " 1M ".to_string(),
            start_time: None,
            end_time: None,
            limit: None,
        }
        .try_into_query()
        .expect("query should be valid");

        assert_eq!(query.source, "binance");
        assert_eq!(query.symbol, "BTCUSDT");
        assert_eq!(query.interval, "1m");
        assert_eq!(query.limit, DEFAULT_LIMIT);
    }

    #[test]
    fn query_params_reject_large_limit() {
        let error = KlineQueryParams {
            source: None,
            symbol: "BTCUSDT".to_string(),
            interval: "1m".to_string(),
            start_time: None,
            end_time: None,
            limit: Some(MAX_LIMIT + 1),
        }
        .try_into_query()
        .expect_err("large limit should fail");

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[test]
    fn query_params_reject_inverted_time_range() {
        let error = KlineQueryParams {
            source: None,
            symbol: "BTCUSDT".to_string(),
            interval: "1m".to_string(),
            start_time: Some(2_000),
            end_time: Some(1_000),
            limit: None,
        }
        .try_into_query()
        .expect_err("inverted time range should fail");

        assert!(matches!(error, ApiError::BadRequest(_)));
    }
}
