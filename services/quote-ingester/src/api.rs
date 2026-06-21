use std::{str::FromStr, time::Duration};

use chrono::{DateTime, TimeZone, Utc};
use reqwest::Url;
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use thiserror::Error;

use crate::models::Kline;

/// Binance K 线接口客户端。
///
/// 这个结构体保存两个东西：
/// - `http`：可复用的 reqwest HTTP 客户端。
/// - `base_url`：API 域名，比如 `https://www.binance.com`。
#[derive(Clone)]
pub struct QuoteApiClient {
    /// reqwest 客户端内部会复用连接，Clone 成本很低。
    http: reqwest::Client,
    /// 预先解析好的 URL，避免每次请求都重新校验基础地址。
    base_url: Url,
}

/// 一次 K 线请求需要的参数。
///
/// 这里使用生命周期 `'a`，因为这些字段只是借用调用方已有的字符串，
/// 不需要为每次请求重新分配 `String`。
#[derive(Debug, Clone)]
pub struct KlineFetchRequest<'a> {
    /// 数据源名称，会写入数据库。
    pub source: &'a str,
    /// 交易对，例如 `BTCUSDT`。
    pub symbol: &'a str,
    /// 周期，例如 `1m`。
    pub interval: &'a str,
    /// 可选开始时间。`None` 表示不传 startTime。
    pub start_time: Option<DateTime<Utc>>,
    /// 可选结束时间。`None` 表示不传 endTime。
    pub end_time: Option<DateTime<Utc>>,
    /// 单次请求最多返回多少根。
    pub limit: u32,
}

/// 调用行情接口时可能出现的错误。
#[derive(Debug, Error)]
pub enum QuoteApiError {
    /// 基础 URL 不是合法 URL。
    #[error("invalid quote api base url: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    /// 拼接具体接口路径失败。
    #[error("failed to build quote api request url: {0}")]
    BuildUrl(url::ParseError),
    /// 网络请求、HTTP 状态码或超时错误。
    #[error("quote api request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// JSON 解析失败。
    #[error("quote api response json is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// Binance 返回了错误对象，比如 `{ "code": -1121, "msg": "Invalid symbol." }`。
    #[error("quote api returned code={code}, msg={message}")]
    Api { code: i32, message: String },
    /// 返回既不是 K 线数组，也不是错误对象。
    #[error("quote api response should be an array or error object")]
    InvalidResponse,
    /// K 线时间戳不是合法毫秒时间戳。
    #[error("invalid kline timestamp millis: {0}")]
    InvalidTimestamp(i64),
}

impl QuoteApiClient {
    /// 创建行情接口客户端。
    ///
    /// `timeout` 作用于整个 HTTP 请求，避免接口卡住时 worker 永久等待。
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, QuoteApiError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("mini-micro-rs quote-ingester/0.1")
            .build()?;
        Ok(Self {
            http,
            base_url: Url::parse(base_url)?,
        })
    }

    /// 拉取一批 K 线，并转换成服务内部统一的 `Kline` 模型。
    pub async fn fetch_klines(
        &self,
        request: KlineFetchRequest<'_>,
    ) -> Result<Vec<Kline>, QuoteApiError> {
        // Binance uiKlines 路径固定，base_url 只负责域名。
        let mut url = self
            .base_url
            .join("/api/v3/uiKlines")
            .map_err(QuoteApiError::BuildUrl)?;

        {
            // query_pairs_mut 用来安全拼接 query string，避免自己手写 `?a=b&c=d`。
            let mut query = url.query_pairs_mut();
            query.append_pair("symbol", request.symbol);
            query.append_pair("interval", request.interval);
            query.append_pair("limit", &request.limit.to_string());
            // Binance 支持 timeZone 参数。这里保持和你验证接口时一致，使用东八区。
            query.append_pair("timeZone", "08:00");
            if let Some(start_time) = request.start_time {
                query.append_pair("startTime", &start_time.timestamp_millis().to_string());
            }
            if let Some(end_time) = request.end_time {
                query.append_pair("endTime", &end_time.timestamp_millis().to_string());
            }
        }

        // 先解析成 serde_json::Value，是因为 Binance 成功时返回数组，失败时返回对象。
        let value = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;

        // 如果是对象，就当成 Binance 错误响应处理。
        if let Some(error) = value.as_object() {
            let code = error
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default() as i32;
            let message = error
                .get("msg")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            return Err(QuoteApiError::Api { code, message });
        }

        // 成功响应是数组：Vec<ApiKline>。
        // ApiKline 自己实现了 Deserialize，用来解析 Binance 的数组格式。
        let response =
            serde_json::from_value::<Vec<ApiKline>>(value).map_err(|error| {
                match error.classify() {
                    serde_json::error::Category::Data | serde_json::error::Category::Syntax => {
                        QuoteApiError::Json(error)
                    }
                    _ => QuoteApiError::InvalidResponse,
                }
            })?;

        // 把接口模型转换成数据库/业务统一使用的 Kline 模型。
        response
            .into_iter()
            .map(|item| item.into_kline(request.source, request.symbol, request.interval))
            .collect()
    }
}

/// Binance 返回的一根 K 线。
///
/// Binance 的返回不是对象，而是数组：
/// `[openTime, open, high, low, close, volume, closeTime, quoteAssetVolume, ...]`
///
/// 所以这里没有直接 `derive(Deserialize)`，而是下面手写了解析逻辑。
#[derive(Debug, Clone)]
struct ApiKline {
    /// 开盘时间，毫秒时间戳。
    open_time_millis: i64,
    /// 开盘价。
    open_price: Decimal,
    /// 最高价。
    high_price: Decimal,
    /// 最低价。
    low_price: Decimal,
    /// 收盘价。
    close_price: Decimal,
    /// 基础币成交量。
    base_volume: Decimal,
    /// 收盘时间，当前不入库，但保留字段便于以后使用。
    #[allow(dead_code)]
    close_time_millis: i64,
    /// 计价币成交额。
    quote_volume: Decimal,
}

impl<'de> Deserialize<'de> for ApiKline {
    /// 手写 serde 反序列化，把 Binance 数组转换成有名字的字段。
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // 先把数组解析成 Vec<Value>，再按下标取字段。
        let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
        if values.len() < 8 {
            return Err(de::Error::custom(format!(
                "expected at least 8 kline fields, got {}",
                values.len()
            )));
        }

        Ok(Self {
            open_time_millis: i64_from_value(&values[0])?,
            open_price: decimal_from_value(&values[1])?,
            high_price: decimal_from_value(&values[2])?,
            low_price: decimal_from_value(&values[3])?,
            close_price: decimal_from_value(&values[4])?,
            base_volume: decimal_from_value(&values[5])?,
            close_time_millis: i64_from_value(&values[6])?,
            quote_volume: decimal_from_value(&values[7])?,
        })
    }
}

impl ApiKline {
    /// 把接口层模型转换成内部统一 Kline。
    fn into_kline(
        self,
        source: &str,
        symbol: &str,
        interval: &str,
    ) -> Result<Kline, QuoteApiError> {
        // Binance 返回毫秒时间戳，chrono 用 timestamp_millis_opt 安全转换成 UTC 时间。
        let open_time = Utc
            .timestamp_millis_opt(self.open_time_millis)
            .single()
            .ok_or(QuoteApiError::InvalidTimestamp(self.open_time_millis))?;

        Ok(Kline::new(
            source,
            symbol,
            interval,
            open_time,
            self.open_price,
            self.high_price,
            self.low_price,
            self.close_price,
            self.base_volume,
            self.quote_volume,
            // 直接从交易所拉到的一根 K 线，来源数量就是 1，且认为它本身完整。
            1,
            true,
        ))
    }
}

/// 从 JSON 值解析 Decimal。
///
/// 价格和数量不能用 f64 保存，否则会有二进制浮点误差。
/// Binance 返回字符串形式的小数，这里统一解析成 Decimal。
fn decimal_from_value<E>(value: &serde_json::Value) -> Result<Decimal, E>
where
    E: de::Error,
{
    match value {
        serde_json::Value::Number(number) => {
            Decimal::from_str(&number.to_string()).map_err(de::Error::custom)
        }
        serde_json::Value::String(value) => Decimal::from_str(value).map_err(de::Error::custom),
        other => Err(de::Error::custom(format!(
            "expected decimal number or string, got {other}"
        ))),
    }
}

/// 从 JSON 值解析 i64。
///
/// 用于解析 openTime/closeTime 这类毫秒时间戳。
fn i64_from_value<E>(value: &serde_json::Value) -> Result<i64, E>
where
    E: de::Error,
{
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| de::Error::custom(format!("expected i64, got {number}"))),
        serde_json::Value::String(value) => value.parse::<i64>().map_err(de::Error::custom),
        other => Err(de::Error::custom(format!(
            "expected integer number or string, got {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_response() {
        let response = serde_json::from_str::<Vec<ApiKline>>(
            r#"
            [
              [
                1782029460000,
                "64127.02000000",
                "64127.03000000",
                "64127.02000000",
                "64127.03000000",
                "0.63157000",
                1782029519999,
                "40500.70259170",
                15,
                "0.05703000",
                "3657.16452090",
                "0"
              ]
            ]
            "#,
        )
        .expect("response should parse");

        let kline = response[0]
            .clone()
            .into_kline("binance", "BTCUSDT", "1m")
            .expect("kline should convert");

        assert_eq!(kline.source, "binance");
        assert_eq!(kline.symbol, "BTCUSDT");
        assert_eq!(kline.interval, "1m");
        assert_eq!(kline.open_time.timestamp_millis(), 1782029460000);
        assert_eq!(
            kline.open_price,
            Decimal::from_str("64127.02000000").unwrap()
        );
        assert_eq!(kline.base_volume, Decimal::from_str("0.63157000").unwrap());
        assert_eq!(
            kline.quote_volume,
            Decimal::from_str("40500.70259170").unwrap()
        );
        assert!(kline.is_complete);
    }
}
