use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, QueryBuilder};
use thiserror::Error;

use crate::models::Kline;

/// Postgres 版 K 线存储。
///
/// 这个结构体只有一个字段：`PgPool`。
/// `PgPool` 是数据库连接池，可以被 clone，并在多个异步任务之间共享。
#[derive(Clone)]
pub struct PostgresKlineStore {
    /// sqlx 的 Postgres 连接池。
    pool: PgPool,
}

/// 数据库层错误。
#[derive(Debug, Error)]
pub enum StoreError {
    /// 直接包装 sqlx 返回的数据库错误。
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl PostgresKlineStore {
    /// 创建数据库连接池。
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// 从已有连接池创建 store。
    ///
    /// 当前主要给测试或未来复用场景准备。
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 暴露连接池引用。
    ///
    /// main.rs 需要拿这个 pool 去执行 sqlx migration。
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 查询某个 source/symbol/interval 已经保存到的最新开盘时间。
    ///
    /// worker 用它判断实时增量同步从哪里开始。
    pub async fn latest_open_time(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
    ) -> Result<Option<DateTime<Utc>>, StoreError> {
        sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            r#"
            SELECT max(open_time)
            FROM klines
            WHERE source = $1 AND symbol = $2 AND interval = $3
            "#,
        )
        .bind(source)
        .bind(symbol)
        .bind(interval)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::Database)
    }

    /// 加载某个时间窗口内的 K 线。
    ///
    /// 注意这里使用 `[start_time, end_time)`：
    /// - 包含 start_time
    /// - 不包含 end_time
    ///
    /// 这样连续窗口拼接时不会重复计算边界那一根。
    pub async fn load_klines(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<Kline>, StoreError> {
        sqlx::query_as::<_, Kline>(
            r#"
            SELECT source, symbol, interval, open_time, open_price, high_price,
                   low_price, close_price, base_volume, quote_volume,
                   source_count, is_complete
            FROM klines
            WHERE source = $1
              AND symbol = $2
              AND interval = $3
              AND open_time >= $4
              AND open_time < $5
            ORDER BY open_time ASC
            "#,
        )
        .bind(source)
        .bind(symbol)
        .bind(interval)
        .bind(start_time)
        .bind(end_time)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::Database)
    }

    /// 批量 upsert K 线。
    ///
    /// upsert 的意思是：
    /// - 不存在就 INSERT。
    /// - 已存在就 UPDATE。
    ///
    /// `klines` 表的唯一键是 `(source, symbol, interval, open_time)`，
    /// 所以同一根 K 线反复拉取不会产生重复行。
    pub async fn upsert_klines(&self, klines: &[Kline]) -> Result<u64, StoreError> {
        if klines.is_empty() {
            return Ok(0);
        }

        // QueryBuilder 适合动态拼批量 INSERT。
        // 这里不用字符串拼值，而是用 push_bind，让 sqlx 做参数绑定，避免 SQL 注入。
        let mut builder = QueryBuilder::<Postgres>::new(
            r#"
            INSERT INTO klines (
                source, symbol, interval, open_time, open_price, high_price,
                low_price, close_price, base_volume, quote_volume,
                source_count, is_complete
            )
            "#,
        );

        // push_values 会为每个 Kline 生成一组绑定参数。
        builder.push_values(klines, |mut row, kline| {
            row.push_bind(&kline.source)
                .push_bind(&kline.symbol)
                .push_bind(&kline.interval)
                .push_bind(kline.open_time)
                .push_bind(kline.open_price)
                .push_bind(kline.high_price)
                .push_bind(kline.low_price)
                .push_bind(kline.close_price)
                .push_bind(kline.base_volume)
                .push_bind(kline.quote_volume)
                .push_bind(kline.source_count)
                .push_bind(kline.is_complete);
        });

        // 冲突时更新 OHLCV、完整性字段和 updated_at。
        // 这对未收盘 K 线很重要，因为交易所会持续更新最新一根 K 线。
        builder.push(
            r#"
            ON CONFLICT (source, symbol, interval, open_time)
            DO UPDATE SET
                open_price = EXCLUDED.open_price,
                high_price = EXCLUDED.high_price,
                low_price = EXCLUDED.low_price,
                close_price = EXCLUDED.close_price,
                base_volume = EXCLUDED.base_volume,
                quote_volume = EXCLUDED.quote_volume,
                source_count = EXCLUDED.source_count,
                is_complete = EXCLUDED.is_complete,
                updated_at = now()
            "#,
        );

        let result = builder.build().execute(&self.pool).await?;
        // rows_affected 返回本次 INSERT/UPDATE 影响的行数。
        Ok(result.rows_affected())
    }
}
