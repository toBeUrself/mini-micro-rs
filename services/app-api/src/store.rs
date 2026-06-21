use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{postgres::PgPoolOptions, FromRow, PgPool, Postgres, QueryBuilder};
use thiserror::Error;

/// app-api 使用的 Postgres 存储对象。
///
/// 目前它只查询 K 线表；后面查询 users、分析结果等表，也可以继续在这里扩展。
#[derive(Clone)]
pub struct PostgresAppStore {
    /// 数据库连接池。
    ///
    /// `PgPool` 本身可以 clone，clone 出来的对象仍然共享同一个连接池。
    pool: PgPool,
}

/// 数据库层错误。
#[derive(Debug, Error)]
pub enum StoreError {
    /// sqlx 返回的底层数据库错误。
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// 查询 K 线时使用的内部参数。
///
/// HTTP 层会先校验 query string，然后转换成这个结构体。
#[derive(Debug, Clone)]
pub struct KlineQuery {
    /// 数据来源，比如 `binance`。
    pub source: String,
    /// 交易对，比如 `BTCUSDT`。
    pub symbol: String,
    /// K 线周期，比如 `1m`、`5m`、`30m`。
    pub interval: String,
    /// 查询开始时间，包含这根 K 线。
    pub start_time: Option<DateTime<Utc>>,
    /// 查询结束时间，不包含这根 K 线。
    pub end_time: Option<DateTime<Utc>>,
    /// 最大返回数量。
    pub limit: i64,
}

/// 数据库里的 K 线行。
///
/// `FromRow` 表示 sqlx 可以把查询结果自动填充到这个结构体。
#[derive(Debug, Clone, FromRow)]
pub struct KlineRow {
    /// 数据来源。
    pub source: String,
    /// 交易对。
    pub symbol: String,
    /// 周期。
    pub interval: String,
    /// 开盘时间。
    pub open_time: DateTime<Utc>,
    /// 开盘价。
    pub open_price: Decimal,
    /// 最高价。
    pub high_price: Decimal,
    /// 最低价。
    pub low_price: Decimal,
    /// 收盘价。
    pub close_price: Decimal,
    /// 基础币成交量。
    pub base_volume: Decimal,
    /// 计价币成交额。
    pub quote_volume: Decimal,
    /// 聚合时使用了多少根源 K 线。
    pub source_count: i32,
    /// 这根 K 线是否完整。
    pub is_complete: bool,
}

/// 某个 source/symbol/interval 的数据概览。
#[derive(Debug, Clone, FromRow)]
pub struct KlineMarketSummary {
    /// 数据来源。
    pub source: String,
    /// 交易对。
    pub symbol: String,
    /// 周期。
    pub interval: String,
    /// 数据库里最早一根 K 线的开盘时间。
    pub start_time: DateTime<Utc>,
    /// 数据库里最新一根 K 线的开盘时间。
    pub end_time: DateTime<Utc>,
    /// 总行数。
    pub row_count: i64,
}

impl PostgresAppStore {
    /// 创建数据库连接池。
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// 暴露连接池引用。
    ///
    /// main.rs 需要拿它执行 migration。
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 查询 K 线列表。
    ///
    /// 这里先按 `open_time DESC` 查最近 N 条，保证能用到降序索引；
    /// 查出来后再在 Rust 里反转成升序，方便前端画 K 线图。
    pub async fn query_klines(&self, query: KlineQuery) -> Result<Vec<KlineRow>, StoreError> {
        let mut builder = QueryBuilder::<Postgres>::new(
            r#"
            SELECT source, symbol, interval, open_time, open_price, high_price,
                   low_price, close_price, base_volume, quote_volume,
                   source_count, is_complete
            FROM klines
            WHERE source =
            "#,
        );

        builder
            .push_bind(&query.source)
            .push(" AND symbol = ")
            .push_bind(&query.symbol)
            .push(" AND interval = ")
            .push_bind(&query.interval);

        if let Some(start_time) = query.start_time {
            builder.push(" AND open_time >= ").push_bind(start_time);
        }

        if let Some(end_time) = query.end_time {
            builder.push(" AND open_time < ").push_bind(end_time);
        }

        builder
            .push(" ORDER BY open_time DESC LIMIT ")
            .push_bind(query.limit);

        let mut rows = builder
            .build_query_as::<KlineRow>()
            .fetch_all(&self.pool)
            .await?;
        rows.reverse();
        Ok(rows)
    }

    /// 查询当前数据库里有哪些 K 线数据。
    pub async fn list_kline_markets(&self) -> Result<Vec<KlineMarketSummary>, StoreError> {
        sqlx::query_as::<_, KlineMarketSummary>(
            r#"
            SELECT source,
                   symbol,
                   interval,
                   min(open_time) AS start_time,
                   max(open_time) AS end_time,
                   count(*)::BIGINT AS row_count
            FROM klines
            GROUP BY source, symbol, interval
            ORDER BY source ASC, symbol ASC, interval ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::Database)
    }
}
