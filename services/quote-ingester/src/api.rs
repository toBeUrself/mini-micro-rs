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
    #[error("quote api returned rc={rc}, mc={message}")]
    Api { rc: i32, message: String },
    #[error("invalid kline timestamp millis: {0}")]
    InvalidTimestamp(i64),
}

impl QuoteApiClient {
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, QuoteApiError> {
        let http = reqwest::Client::builder().timeout(timeout).build()?;
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
            .join("/sapi/v1/quote/public/kline")
            .map_err(QuoteApiError::BuildUrl)?;

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("symbol", request.symbol);
            query.append_pair("interval", request.interval);
            query.append_pair("limit", &request.limit.to_string());
            if let Some(start_time) = request.start_time {
                query.append_pair("startTime", &start_time.timestamp_millis().to_string());
            }
            if let Some(end_time) = request.end_time {
                query.append_pair("endTime", &end_time.timestamp_millis().to_string());
            }
        }

        let response = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<KlineResponse>()
            .await?;

        if response.rc != 0 {
            return Err(QuoteApiError::Api {
                rc: response.rc,
                message: response.mc,
            });
        }

        response
            .result
            .into_iter()
            .map(|item| item.into_kline(request.source, request.symbol, request.interval))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct KlineResponse {
    rc: i32,
    mc: String,
    #[allow(dead_code)]
    ma: Vec<serde_json::Value>,
    result: Vec<ApiKline>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiKline {
    t: i64,
    #[serde(rename = "o", deserialize_with = "deserialize_decimal")]
    open_price: Decimal,
    #[serde(rename = "h", deserialize_with = "deserialize_decimal")]
    high_price: Decimal,
    #[serde(rename = "l", deserialize_with = "deserialize_decimal")]
    low_price: Decimal,
    #[serde(rename = "c", deserialize_with = "deserialize_decimal")]
    close_price: Decimal,
    #[serde(rename = "q", deserialize_with = "deserialize_decimal")]
    base_volume: Decimal,
    #[serde(rename = "v", deserialize_with = "deserialize_decimal")]
    quote_volume: Decimal,
}

impl ApiKline {
    fn into_kline(
        self,
        source: &str,
        symbol: &str,
        interval: &str,
    ) -> Result<Kline, QuoteApiError> {
        let open_time = Utc
            .timestamp_millis_opt(self.t)
            .single()
            .ok_or(QuoteApiError::InvalidTimestamp(self.t))?;

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

fn deserialize_decimal<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => {
            Decimal::from_str(&number.to_string()).map_err(de::Error::custom)
        }
        serde_json::Value::String(value) => Decimal::from_str(&value).map_err(de::Error::custom),
        other => Err(de::Error::custom(format!(
            "expected decimal number or string, got {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_response() {
        let response = serde_json::from_str::<KlineResponse>(
            r#"
            {
              "rc": 0,
              "mc": "SUCCESS",
              "ma": [],
              "result": [
                {"t":1782021300000,"o":64255.00,"h":64255.99,"l":64255.00,"c":64255.99,"q":0.043950,"v":2804.18}
              ]
            }
            "#,
        )
        .expect("response should parse");

        let kline = response.result[0]
            .clone()
            .into_kline("azverse", "btc_usdt", "1m")
            .expect("kline should convert");

        assert_eq!(kline.source, "azverse");
        assert_eq!(kline.symbol, "btc_usdt");
        assert_eq!(kline.interval, "1m");
        assert_eq!(kline.open_time.timestamp_millis(), 1782021300000);
        assert_eq!(kline.open_price, Decimal::from_str("64255.00").unwrap());
        assert_eq!(kline.base_volume, Decimal::from_str("0.043950").unwrap());
        assert!(kline.is_complete);
    }
}
