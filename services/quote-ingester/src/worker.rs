use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;
use tokio::task::JoinSet;

use crate::{
    aggregate::{aggregate_klines, bucket_start, AggregateError},
    api::{KlineFetchRequest, QuoteApiClient, QuoteApiError},
    config::{interval_minutes, MarketConfig, ResolvedConfig},
    models::Kline,
    store::{PostgresKlineStore, StoreError},
};

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("quote api error: {0}")]
    Api(#[from] QuoteApiError),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("aggregation error: {0}")]
    Aggregate(#[from] AggregateError),
    #[error("invalid interval: {0}")]
    InvalidInterval(String),
    #[error("invalid time delta")]
    InvalidTimeDelta,
    #[error("market task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("ctrl-c handler failed: {0}")]
    Signal(std::io::Error),
}

pub async fn run(config: ResolvedConfig) -> Result<(), WorkerError> {
    let store = PostgresKlineStore::connect(&config.database_url)
        .await
        .map_err(StoreError::Database)?;
    let client = QuoteApiClient::new(&config.api_base_url, config.api_timeout)?;

    let mut tasks = JoinSet::new();
    for market in config.markets {
        let store = store.clone();
        let client = client.clone();
        let source = config.source.clone();
        tasks.spawn(async move { run_market_loop(store, client, source, market).await });
    }

    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(WorkerError::Signal)?;
                tracing::info!("quote ingester shutting down");
                tasks.abort_all();
                return Ok(());
            }
            result = tasks.join_next() => {
                match result {
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(error))) => return Err(error),
                    Some(Err(error)) => return Err(WorkerError::Join(error)),
                    None => return Ok(()),
                }
            }
        }
    }
}

async fn run_market_loop(
    store: PostgresKlineStore,
    client: QuoteApiClient,
    source: String,
    market: MarketConfig,
) -> Result<(), WorkerError> {
    tracing::info!(
        source = %source,
        symbol = %market.symbol,
        interval = %market.source_interval,
        backfill_days = market.backfill_days,
        "starting quote market worker"
    );

    backfill_market(&store, &client, &source, &market).await?;

    let mut ticker = tokio::time::interval(Duration::from_secs(market.poll_seconds));
    loop {
        ticker.tick().await;
        if let Err(error) = sync_latest_once(&store, &client, &source, &market).await {
            tracing::warn!(
                source = %source,
                symbol = %market.symbol,
                interval = %market.source_interval,
                error = %error,
                "latest quote sync failed"
            );
        }
    }
}

async fn backfill_market(
    store: &PostgresKlineStore,
    client: &QuoteApiClient,
    source: &str,
    market: &MarketConfig,
) -> Result<(), WorkerError> {
    let source_minutes = interval_minutes(&market.source_interval)
        .ok_or_else(|| WorkerError::InvalidInterval(market.source_interval.clone()))?;
    let source_delta = minutes_delta(source_minutes)?;
    let end_time = Utc::now();
    let start_time = end_time
        .checked_sub_signed(TimeDelta::days(market.backfill_days as i64))
        .ok_or(WorkerError::InvalidTimeDelta)?;
    let cursor_end = end_time
        .checked_add_signed(source_delta)
        .ok_or(WorkerError::InvalidTimeDelta)?;

    tracing::info!(
        source = %source,
        symbol = %market.symbol,
        interval = %market.source_interval,
        start_time = %start_time,
        end_time = %end_time,
        "backfilling quote history"
    );

    fetch_store_and_aggregate_backward(
        store, client, source, market, cursor_end, start_time, end_time,
    )
    .await?;
    Ok(())
}

async fn sync_latest_once(
    store: &PostgresKlineStore,
    client: &QuoteApiClient,
    source: &str,
    market: &MarketConfig,
) -> Result<(), WorkerError> {
    let source_minutes = interval_minutes(&market.source_interval)
        .ok_or_else(|| WorkerError::InvalidInterval(market.source_interval.clone()))?;
    let source_delta = minutes_delta(source_minutes)?;
    let latest = store
        .latest_open_time(source, &market.symbol, &market.source_interval)
        .await?;

    match latest {
        Some(latest) => {
            let start_time = latest
                .checked_sub_signed(source_delta)
                .ok_or(WorkerError::InvalidTimeDelta)?;
            fetch_store_and_aggregate_forward(
                store, client, source, market, start_time, None, start_time,
            )
            .await?;
        }
        None => {
            let end_time = Utc::now();
            let start_time = end_time
                .checked_sub_signed(TimeDelta::days(market.backfill_days as i64))
                .ok_or(WorkerError::InvalidTimeDelta)?;
            let cursor_end = end_time
                .checked_add_signed(source_delta)
                .ok_or(WorkerError::InvalidTimeDelta)?;
            fetch_store_and_aggregate_backward(
                store, client, source, market, cursor_end, start_time, end_time,
            )
            .await?;
        }
    };
    Ok(())
}

async fn fetch_store_and_aggregate_forward(
    store: &PostgresKlineStore,
    client: &QuoteApiClient,
    source: &str,
    market: &MarketConfig,
    mut cursor: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    keep_from: DateTime<Utc>,
) -> Result<(), WorkerError> {
    let source_minutes = interval_minutes(&market.source_interval)
        .ok_or_else(|| WorkerError::InvalidInterval(market.source_interval.clone()))?;
    let source_delta = minutes_delta(source_minutes)?;

    loop {
        let mut fetched = client
            .fetch_klines(KlineFetchRequest {
                source,
                symbol: &market.symbol,
                interval: &market.source_interval,
                start_time: Some(cursor),
                end_time: None,
                limit: market.limit,
            })
            .await?;

        if fetched.is_empty() {
            tracing::info!(
                source = %source,
                symbol = %market.symbol,
                interval = %market.source_interval,
                cursor = %cursor,
                "quote api returned no forward klines"
            );
            break;
        }

        fetched.sort_by_key(|kline| kline.open_time);
        let last_fetched_time = fetched[fetched.len() - 1].open_time;

        let filtered = filter_fetched_klines(fetched, keep_from, end_time);
        store_and_aggregate_batch(store, source, market, source_minutes, filtered).await?;

        if let Some(end_time) = end_time {
            if last_fetched_time >= end_time {
                break;
            }
        }

        if last_fetched_time <= cursor {
            tracing::warn!(
                source = %source,
                symbol = %market.symbol,
                interval = %market.source_interval,
                cursor = %cursor,
                last_fetched_time = %last_fetched_time,
                "quote api did not advance cursor"
            );
            break;
        }

        cursor = last_fetched_time;
        if last_fetched_time
            .checked_add_signed(source_delta)
            .ok_or(WorkerError::InvalidTimeDelta)?
            > Utc::now()
        {
            break;
        }
    }

    Ok(())
}

async fn fetch_store_and_aggregate_backward(
    store: &PostgresKlineStore,
    client: &QuoteApiClient,
    source: &str,
    market: &MarketConfig,
    mut cursor_end: DateTime<Utc>,
    keep_from: DateTime<Utc>,
    keep_until: DateTime<Utc>,
) -> Result<(), WorkerError> {
    let source_minutes = interval_minutes(&market.source_interval)
        .ok_or_else(|| WorkerError::InvalidInterval(market.source_interval.clone()))?;

    loop {
        let mut fetched = client
            .fetch_klines(KlineFetchRequest {
                source,
                symbol: &market.symbol,
                interval: &market.source_interval,
                start_time: None,
                end_time: Some(cursor_end),
                limit: market.limit,
            })
            .await?;

        if fetched.is_empty() {
            tracing::info!(
                source = %source,
                symbol = %market.symbol,
                interval = %market.source_interval,
                cursor_end = %cursor_end,
                "quote api returned no backward klines"
            );
            break;
        }

        fetched.sort_by_key(|kline| kline.open_time);
        let oldest_fetched_time = fetched[0].open_time;

        let filtered = filter_fetched_klines(fetched, keep_from, Some(keep_until));
        store_and_aggregate_batch(store, source, market, source_minutes, filtered).await?;

        if oldest_fetched_time <= keep_from {
            break;
        }

        if oldest_fetched_time >= cursor_end {
            tracing::warn!(
                source = %source,
                symbol = %market.symbol,
                interval = %market.source_interval,
                cursor_end = %cursor_end,
                oldest_fetched_time = %oldest_fetched_time,
                "quote api did not move backward cursor"
            );
            break;
        }

        cursor_end = oldest_fetched_time;
    }

    Ok(())
}

async fn store_and_aggregate_batch(
    store: &PostgresKlineStore,
    source: &str,
    market: &MarketConfig,
    source_minutes: i64,
    filtered: Vec<Kline>,
) -> Result<(), WorkerError> {
    if filtered.is_empty() {
        return Ok(());
    }

    let min_time = filtered[0].open_time;
    let max_time = filtered[filtered.len() - 1].open_time;

    let rows = store.upsert_klines(&filtered).await?;
    tracing::info!(
        source = %source,
        symbol = %market.symbol,
        interval = %market.source_interval,
        rows,
        min_time = %min_time,
        max_time = %max_time,
        "stored source klines"
    );

    aggregate_derived_intervals(store, source, market, source_minutes, min_time, max_time).await?;

    Ok(())
}

fn filter_fetched_klines(
    fetched: Vec<Kline>,
    keep_from: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
) -> Vec<Kline> {
    fetched
        .into_iter()
        .filter(|kline| kline.open_time >= keep_from)
        .filter(|kline| end_time.map_or(true, |end_time| kline.open_time <= end_time))
        .collect()
}

async fn aggregate_derived_intervals(
    store: &PostgresKlineStore,
    source: &str,
    market: &MarketConfig,
    source_minutes: i64,
    min_time: DateTime<Utc>,
    max_time: DateTime<Utc>,
) -> Result<(), WorkerError> {
    for target_interval in &market.derived_intervals {
        let target_minutes = interval_minutes(target_interval)
            .ok_or_else(|| WorkerError::InvalidInterval(target_interval.clone()))?;
        let target_delta = minutes_delta(target_minutes)?;
        let query_start = bucket_start(min_time, target_minutes)?;
        let query_end = bucket_start(max_time, target_minutes)?
            .checked_add_signed(target_delta)
            .ok_or(WorkerError::InvalidTimeDelta)?;

        let source_rows = store
            .load_klines(
                source,
                &market.symbol,
                &market.source_interval,
                query_start,
                query_end,
            )
            .await?;
        let aggregated = aggregate_klines(&source_rows, target_interval)?;
        let rows = store.upsert_klines(&aggregated).await?;

        tracing::info!(
            source = %source,
            symbol = %market.symbol,
            source_interval = %market.source_interval,
            target_interval = %target_interval,
            source_minutes,
            rows,
            query_start = %query_start,
            query_end = %query_end,
            "stored derived klines"
        );
    }

    Ok(())
}

fn minutes_delta(minutes: i64) -> Result<TimeDelta, WorkerError> {
    TimeDelta::try_minutes(minutes).ok_or(WorkerError::InvalidTimeDelta)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn filters_fetch_window_boundaries() {
        let before = Utc.with_ymd_and_hms(2026, 6, 21, 9, 59, 0).unwrap();
        let start = Utc.with_ymd_and_hms(2026, 6, 21, 10, 0, 0).unwrap();
        let inside = Utc.with_ymd_and_hms(2026, 6, 21, 10, 1, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 6, 21, 10, 2, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 6, 21, 10, 3, 0).unwrap();

        let rows = vec![
            test_kline(before),
            test_kline(start),
            test_kline(inside),
            test_kline(end),
            test_kline(after),
        ];

        let filtered = filter_fetched_klines(rows, start, Some(end));

        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].open_time, start);
        assert_eq!(filtered[2].open_time, end);
    }

    fn test_kline(open_time: DateTime<Utc>) -> Kline {
        Kline::new(
            "binance",
            "BTCUSDT",
            "1m",
            open_time,
            rust_decimal::Decimal::ONE,
            rust_decimal::Decimal::ONE,
            rust_decimal::Decimal::ONE,
            rust_decimal::Decimal::ONE,
            rust_decimal::Decimal::ONE,
            rust_decimal::Decimal::ONE,
            1,
            true,
        )
    }
}
