use std::{str::FromStr, time::Duration};

use chrono::{DateTime, TimeZone, Utc};
use reqwest::Url;
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use thiserror::Error;

use crate::models::Kline;

#[derive(Clone)]
pub struct QuoteApiClient {
    http: reqwest::Client,
    base_url: Url,
}

#[derive(Debug, Clone)]
pub struct KlineFetchRequest<'a> {
    pub source: &'a str,
    pub symbol: &'a str,
    pub interval: &'a str,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub limit: u32,
}

#[derive(Debug, Error)]
pub enum QuoteApiError {
    #[error("invalid quote api base url: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("failed to build quote api request url: {0}")]
    BuildUrl(url::ParseError),
    #[error("quote api request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("quote api response json is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("quote api returned code={code}, msg={message}")]
    Api { code: i32, message: String },
    #[error("quote api response should be an array or error object")]
    InvalidResponse,
    #[error("invalid kline timestamp millis: {0}")]
    InvalidTimestamp(i64),
}

impl QuoteApiClient {
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

    pub async fn fetch_klines(
        &self,
        request: KlineFetchRequest<'_>,
    ) -> Result<Vec<Kline>, QuoteApiError> {
        let mut url = self
            .base_url
            .join("/api/v3/uiKlines")
            .map_err(QuoteApiError::BuildUrl)?;

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("symbol", request.symbol);
            query.append_pair("interval", request.interval);
            query.append_pair("limit", &request.limit.to_string());
            query.append_pair("timeZone", "08:00");
            if let Some(start_time) = request.start_time {
                query.append_pair("startTime", &start_time.timestamp_millis().to_string());
            }
            if let Some(end_time) = request.end_time {
                query.append_pair("endTime", &end_time.timestamp_millis().to_string());
            }
        }

        let value = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;

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

        let response =
            serde_json::from_value::<Vec<ApiKline>>(value).map_err(|error| {
                match error.classify() {
                    serde_json::error::Category::Data | serde_json::error::Category::Syntax => {
                        QuoteApiError::Json(error)
                    }
                    _ => QuoteApiError::InvalidResponse,
                }
            })?;

        response
            .into_iter()
            .map(|item| item.into_kline(request.source, request.symbol, request.interval))
            .collect()
    }
}

#[derive(Debug, Clone)]
struct ApiKline {
    open_time_millis: i64,
    open_price: Decimal,
    high_price: Decimal,
    low_price: Decimal,
    close_price: Decimal,
    base_volume: Decimal,
    #[allow(dead_code)]
    close_time_millis: i64,
    quote_volume: Decimal,
}

impl<'de> Deserialize<'de> for ApiKline {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
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
    fn into_kline(
        self,
        source: &str,
        symbol: &str,
        interval: &str,
    ) -> Result<Kline, QuoteApiError> {
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
            1,
            true,
        ))
    }
}

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
