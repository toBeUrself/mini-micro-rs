//! 从 app-api HTTP 接口拉取 K 线数据。

use reqwest::Url;
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;

use crate::models::Kline;

/// K 线读取错误。
#[derive(Debug, Error)]
pub enum KlineReaderError {
    #[error("http request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("api returned error status {0}: {1}")]
    ApiError(u16, String),
    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid kline data: {0}")]
    InvalidData(String),
}

/// app-api K 线响应结构。
#[derive(Debug, Deserialize)]
struct KlinesApiResponse {
    source: String,
    symbol: String,
    interval: String,
    items: Vec<KlineApiItem>,
}

/// 单根 K 线的 API 响应格式（camelCase）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KlineApiItem {
    open_time: i64,
    #[serde(rename = "open")]
    open_price: String,
    #[serde(rename = "high")]
    high_price: String,
    #[serde(rename = "low")]
    low_price: String,
    #[serde(rename = "close")]
    close_price: String,
    base_volume: String,
    quote_volume: Option<String>,
    source_count: Option<i32>,
    is_complete: Option<bool>,
}

impl KlineApiItem {
    fn into_kline(&self, interval: &str) -> Result<Kline, KlineReaderError> {
        Ok(Kline {
            open_time: self.open_time,
            interval: interval.to_string(),
            open: self.open_price.parse().map_err(|e| {
                KlineReaderError::InvalidData(format!("invalid open_price: {e}"))
            })?,
            high: self.high_price.parse().map_err(|e| {
                KlineReaderError::InvalidData(format!("invalid high_price: {e}"))
            })?,
            low: self.low_price.parse().map_err(|e| {
                KlineReaderError::InvalidData(format!("invalid low_price: {e}"))
            })?,
            close: self.close_price.parse().map_err(|e| {
                KlineReaderError::InvalidData(format!("invalid close_price: {e}"))
            })?,
            volume: self.base_volume.parse().map_err(|e| {
                KlineReaderError::InvalidData(format!("invalid base_volume: {e}"))
            })?,
            quote_volume: self
                .quote_volume
                .as_deref()
                .map(|v| v.parse::<f64>())
                .transpose()
                .map_err(|e| KlineReaderError::InvalidData(format!("invalid quote_volume: {e}")))?,
            is_closed: self.is_complete.unwrap_or(true),
        })
    }
}

/// K 线 HTTP 客户端。
#[derive(Clone)]
pub struct KlineReader {
    http: reqwest::Client,
    base_url: Url,
}

impl KlineReader {
    /// 创建 K 线读取器。
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, KlineReaderError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("klines-tools/0.1")
            .build()?;
        Ok(Self {
            http,
            base_url: Url::parse(base_url)?,
        })
    }

    /// 拉取 K 线数据。
    ///
    /// - `source`：数据源，如 "binance"
    /// - `symbol`：交易对，如 "BTCUSDT"
    /// - `interval`：周期，如 "5m"
    /// - `start_time`：开始时间（毫秒时间戳），可选
    /// - `end_time`：结束时间（毫秒时间戳），可选
    /// - `limit`：返回数量，默认 500，最大 1000
    pub async fn fetch_klines(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
        start_time: Option<i64>,
        end_time: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<Kline>, KlineReaderError> {
        let url = self
            .base_url
            .join("/api/v1/public/market/klines")?;

        let mut url = url;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("source", source);
            query.append_pair("symbol", symbol);
            query.append_pair("interval", interval);
            query.append_pair("limit", &limit.unwrap_or(500).to_string());
            if let Some(st) = start_time {
                query.append_pair("startTime", &st.to_string());
            }
            if let Some(et) = end_time {
                query.append_pair("endTime", &et.to_string());
            }
        }

        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(KlineReaderError::ApiError(status.as_u16(), body));
        }

        let api_resp: KlinesApiResponse = resp.json().await?;
        let interval_str = interval.to_string();
        let klines: Result<Vec<_>, _> = api_resp
            .items
            .into_iter()
            .map(|item| item.into_kline(&interval_str))
            .collect();
        klines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kline_item() {
        let item = KlineApiItem {
            open_time: 1710000000000,
            open_price: "67360.10".into(),
            high_price: "67500.00".into(),
            low_price: "67200.00".into(),
            close_price: "67400.00".into(),
            base_volume: "100.5".into(),
            quote_volume: Some("6770000.00".into()),
            source_count: Some(1),
            is_complete: Some(true),
        };
        let kline = item.into_kline("5m").expect("should parse");
        assert_eq!(kline.open_time, 1710000000000);
        assert!((kline.open - 67360.10).abs() < 0.01);
        assert!(kline.is_closed);
    }
}
