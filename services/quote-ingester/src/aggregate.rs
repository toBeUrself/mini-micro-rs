use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::{config::interval_minutes, models::Kline};

#[derive(Debug, Error)]
pub enum AggregateError {
    #[error("invalid source interval: {0}")]
    InvalidSourceInterval(String),
    #[error("invalid target interval: {0}")]
    InvalidTargetInterval(String),
    #[error("target interval {target} must be a multiple of source interval {source_interval}")]
    TargetIntervalNotMultiple {
        source_interval: String,
        target: String,
    },
    #[error("target interval {target} must be longer than source interval {source_interval}")]
    TargetIntervalNotLonger {
        source_interval: String,
        target: String,
    },
    #[error("invalid bucket timestamp millis: {0}")]
    InvalidBucketTimestamp(i64),
}

pub fn aggregate_klines(
    source_klines: &[Kline],
    target_interval: &str,
) -> Result<Vec<Kline>, AggregateError> {
    if source_klines.is_empty() {
        return Ok(Vec::new());
    }

    let source_interval = &source_klines[0].interval;
    let source_minutes = interval_minutes(source_interval)
        .ok_or_else(|| AggregateError::InvalidSourceInterval(source_interval.clone()))?;
    let target_minutes = interval_minutes(target_interval)
        .ok_or_else(|| AggregateError::InvalidTargetInterval(target_interval.to_string()))?;

    if target_minutes <= source_minutes {
        return Err(AggregateError::TargetIntervalNotLonger {
            source_interval: source_interval.clone(),
            target: target_interval.to_string(),
        });
    }
    if target_minutes % source_minutes != 0 {
        return Err(AggregateError::TargetIntervalNotMultiple {
            source_interval: source_interval.clone(),
            target: target_interval.to_string(),
        });
    }

    let expected_count = (target_minutes / source_minutes) as i32;
    let mut buckets: BTreeMap<DateTime<Utc>, Vec<&Kline>> = BTreeMap::new();
    for kline in source_klines {
        let bucket_start = bucket_start(kline.open_time, target_minutes)?;
        buckets.entry(bucket_start).or_default().push(kline);
    }

    let mut aggregated = Vec::with_capacity(buckets.len());
    for (open_time, mut rows) in buckets {
        rows.sort_by_key(|row| row.open_time);
        let first = rows[0];
        let last = rows[rows.len() - 1];

        let high_price = rows
            .iter()
            .map(|row| row.high_price)
            .max()
            .unwrap_or(first.high_price);
        let low_price = rows
            .iter()
            .map(|row| row.low_price)
            .min()
            .unwrap_or(first.low_price);
        let base_volume = rows
            .iter()
            .map(|row| row.base_volume)
            .fold(Decimal::ZERO, |sum, value| sum + value);
        let quote_volume = rows
            .iter()
            .map(|row| row.quote_volume)
            .fold(Decimal::ZERO, |sum, value| sum + value);
        let source_count = rows.len() as i32;

        aggregated.push(Kline::new(
            first.source.clone(),
            first.symbol.clone(),
            target_interval.to_string(),
            open_time,
            first.open_price,
            high_price,
            low_price,
            last.close_price,
            base_volume,
            quote_volume,
            source_count,
            source_count == expected_count,
        ));
    }

    Ok(aggregated)
}

pub fn bucket_start(
    open_time: DateTime<Utc>,
    interval_minutes: i64,
) -> Result<DateTime<Utc>, AggregateError> {
    let interval_millis = interval_minutes * 60 * 1000;
    let timestamp_millis = open_time.timestamp_millis();
    let bucket_millis = timestamp_millis.div_euclid(interval_millis) * interval_millis;
    Utc.timestamp_millis_opt(bucket_millis)
        .single()
        .ok_or(AggregateError::InvalidBucketTimestamp(bucket_millis))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn decimal(value: &str) -> Decimal {
        Decimal::from_str(value).unwrap()
    }

    fn kline(minute: u32, open: &str, high: &str, low: &str, close: &str, volume: &str) -> Kline {
        Kline::new(
            "azverse",
            "btc_usdt",
            "1m",
            Utc.with_ymd_and_hms(2026, 6, 21, 10, minute, 0).unwrap(),
            decimal(open),
            decimal(high),
            decimal(low),
            decimal(close),
            decimal(volume),
            decimal(volume),
            1,
            true,
        )
    }

    #[test]
    fn aggregates_complete_five_minute_window() {
        let source = vec![
            kline(0, "100", "110", "99", "105", "1"),
            kline(1, "105", "112", "101", "108", "2"),
            kline(2, "108", "109", "97", "99", "3"),
            kline(3, "99", "115", "98", "111", "4"),
            kline(4, "111", "113", "100", "102", "5"),
        ];

        let aggregated = aggregate_klines(&source, "5m").unwrap();

        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].interval, "5m");
        assert_eq!(aggregated[0].open_time, source[0].open_time);
        assert_eq!(aggregated[0].open_price, decimal("100"));
        assert_eq!(aggregated[0].high_price, decimal("115"));
        assert_eq!(aggregated[0].low_price, decimal("97"));
        assert_eq!(aggregated[0].close_price, decimal("102"));
        assert_eq!(aggregated[0].base_volume, decimal("15"));
        assert_eq!(aggregated[0].source_count, 5);
        assert!(aggregated[0].is_complete);
    }

    #[test]
    fn marks_partial_window_incomplete() {
        let source = vec![
            kline(0, "100", "110", "99", "105", "1"),
            kline(1, "105", "112", "101", "108", "2"),
        ];

        let aggregated = aggregate_klines(&source, "5m").unwrap();

        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].source_count, 2);
        assert!(!aggregated[0].is_complete);
    }

    #[test]
    fn aligns_buckets_to_utc_boundaries() {
        let open_time = Utc.with_ymd_and_hms(2026, 6, 21, 10, 7, 0).unwrap();
        let bucket = bucket_start(open_time, 5).unwrap();

        assert_eq!(bucket, Utc.with_ymd_and_hms(2026, 6, 21, 10, 5, 0).unwrap());
    }
}
