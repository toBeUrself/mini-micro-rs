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

/// worker 层错误。
///
/// worker 是最上层业务流程，所以它会把 API、数据库、聚合、任务管理等错误统一起来。
#[derive(Debug, Error)]
pub enum WorkerError {
    /// 行情接口错误。
    #[error("quote api error: {0}")]
    Api(#[from] QuoteApiError),
    /// 数据库错误。
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// K 线聚合错误。
    #[error("aggregation error: {0}")]
    Aggregate(#[from] AggregateError),
    /// 配置里的周期无法识别。
    #[error("invalid interval: {0}")]
    InvalidInterval(String),
    /// 时间加减溢出或无效。
    #[error("invalid time delta")]
    InvalidTimeDelta,
    /// 某个 market 子任务异常结束。
    #[error("market task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    /// Ctrl-C 信号监听失败。
    #[error("ctrl-c handler failed: {0}")]
    Signal(std::io::Error),
}

/// 启动 quote-ingester worker。
///
/// 一个配置里可以有多个 market。这里会为每个 market 启动一个异步任务，
/// 让不同交易对或周期可以并行采集。
pub async fn run(config: ResolvedConfig) -> Result<(), WorkerError> {
    let store = PostgresKlineStore::connect(&config.database_url)
        .await
        .map_err(StoreError::Database)?;
    let client = QuoteApiClient::new(&config.api_base_url, config.api_timeout)?;

    // JoinSet 用来管理一组异步任务。
    // 和 Vec<JoinHandle> 相比，它更方便等待“任意一个任务结束”。
    let mut tasks = JoinSet::new();
    for market in config.markets {
        let store = store.clone();
        let client = client.clone();
        let source = config.source.clone();
        tasks.spawn(async move { run_market_loop(store, client, source, market).await });
    }

    // 主循环同时监听两类事件：
    // - 用户按 Ctrl-C。
    // - 某个 market 任务结束或失败。
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
                    // 某个任务正常返回 Ok(())。当前 market loop 理论上不会正常结束。
                    Some(Ok(Ok(()))) => {}
                    // 子任务返回业务错误，直接让整个服务失败。
                    Some(Ok(Err(error))) => return Err(error),
                    // 子任务 panic 或被取消。
                    Some(Err(error)) => return Err(WorkerError::Join(error)),
                    // 没有任何任务了。
                    None => return Ok(()),
                }
            }
        }
    }
}

/// 单个 market 的常驻循环。
///
/// 流程：
/// 1. 启动时先回填历史。
/// 2. 然后按 poll_seconds 周期同步最新数据。
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

    // 启动时先补历史，避免只从当前时间开始采集。
    backfill_market(&store, &client, &source, &market).await?;

    // interval 会按固定间隔 tick。第一次 tick 会很快触发。
    let mut ticker = tokio::time::interval(Duration::from_secs(market.poll_seconds));
    loop {
        ticker.tick().await;
        // 实时同步失败不退出服务，只记录日志，下一轮继续重试。
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

/// 启动时回填历史数据。
///
/// 这里使用“向后翻页”：
/// - 从当前时间作为 endTime 请求最近一页。
/// - 拿到这一页最早的 open_time。
/// - 再用这个时间作为新的 endTime 继续往前请求。
async fn backfill_market(
    store: &PostgresKlineStore,
    client: &QuoteApiClient,
    source: &str,
    market: &MarketConfig,
) -> Result<(), WorkerError> {
    let source_minutes = interval_minutes(&market.source_interval)
        .ok_or_else(|| WorkerError::InvalidInterval(market.source_interval.clone()))?;
    let source_delta = minutes_delta(source_minutes)?;
    // 回填截止到当前时间。
    let end_time = Utc::now();
    // 回填开始时间 = 当前时间 - backfill_days。
    let start_time = end_time
        .checked_sub_signed(TimeDelta::days(market.backfill_days as i64))
        .ok_or(WorkerError::InvalidTimeDelta)?;
    // cursor_end 稍微往后推一个源周期，避免漏掉当前最后一根 K 线。
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

/// 同步最新数据。
///
/// 如果数据库里已经有数据，就从最新时间往前退一个周期再开始拉，
/// 这样可以覆盖更新“最新未收盘 K 线”。
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
        // 已经有历史数据：从最新一根的前一个周期开始向前同步到现在。
        Some(latest) => {
            let start_time = latest
                .checked_sub_signed(source_delta)
                .ok_or(WorkerError::InvalidTimeDelta)?;
            fetch_store_and_aggregate_forward(
                store, client, source, market, start_time, None, start_time,
            )
            .await?;
        }
        // 完全没有数据：退化成一次回填。
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

/// 从 startTime 往未来方向分页拉取并写库。
///
/// 这个函数适合实时增量同步：数据库里已有最新时间，
/// 我们从最新时间附近开始，往当前时间追。
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
        // start_time=Some(cursor) 表示向未来方向拿一页。
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

        // 统一排序，后续取最后一根作为下一轮游标。
        fetched.sort_by_key(|kline| kline.open_time);
        let last_fetched_time = fetched[fetched.len() - 1].open_time;

        // 过滤掉边界之外的数据，再写库和聚合。
        let filtered = filter_fetched_klines(fetched, keep_from, end_time);
        store_and_aggregate_batch(store, source, market, source_minutes, filtered).await?;

        if let Some(end_time) = end_time {
            if last_fetched_time >= end_time {
                break;
            }
        }

        // 如果接口没有把时间往前推进，继续循环会死循环，所以直接停。
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

        // 下一轮从本页最后一根开始继续向前请求。
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

/// 从 endTime 往历史方向分页拉取并写库。
///
/// 这个函数适合历史回填：从现在开始一页一页往过去翻。
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
        // end_time=Some(cursor_end) 表示向历史方向拿一页。
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

        // 排序后第一根就是这一页最早的 K 线。
        fetched.sort_by_key(|kline| kline.open_time);
        let oldest_fetched_time = fetched[0].open_time;

        // keep_from/keep_until 是本次回填真正想要的范围。
        // 接口可能多返回边界之外的数据，所以这里再过滤一次。
        let filtered = filter_fetched_klines(fetched, keep_from, Some(keep_until));
        store_and_aggregate_batch(store, source, market, source_minutes, filtered).await?;

        // 已经翻到回填起点，可以停止。
        if oldest_fetched_time <= keep_from {
            break;
        }

        // 防止接口不推进游标导致死循环。
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

        // 下一轮用这一页最早时间作为新的 endTime，继续往历史方向翻。
        cursor_end = oldest_fetched_time;
    }

    Ok(())
}

/// 写入一批源 K 线，并立即生成派生周期。
///
/// 这里的 `filtered` 已经是经过时间边界过滤后的源 K 线。
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

    // filtered 在调用前已经排序，所以第一根和最后一根可以表示本批次时间范围。
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

    // 源 K 线入库后，再从数据库读取完整窗口并聚合。
    // 这样即使一个 5m 窗口横跨两个接口分页，也能聚合出正确结果。
    aggregate_derived_intervals(store, source, market, source_minutes, min_time, max_time).await?;

    Ok(())
}

/// 过滤接口返回的 K 线，只保留目标时间范围内的数据。
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

/// 根据刚写入的源 K 线时间范围，生成所有 configured derived intervals。
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
        // 聚合必须覆盖完整目标窗口。
        // 例如本批源数据从 10:07 到 10:12，聚合 5m 时要读取 10:05 到 10:15。
        let query_start = bucket_start(min_time, target_minutes)?;
        let query_end = bucket_start(max_time, target_minutes)?
            .checked_add_signed(target_delta)
            .ok_or(WorkerError::InvalidTimeDelta)?;

        // 从数据库重新读取源 K 线，而不是只用当前批次。
        // 这样可以跨分页聚合，也能补齐之前已经存在的数据。
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

/// 把分钟数转换成 chrono 的 TimeDelta。
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
