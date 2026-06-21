use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, QueryBuilder};
use thiserror::Error;

use crate::models::Kline;

#[derive(Clone)]
pub struct PostgresKlineStore {
    pool: PgPool,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl PostgresKlineStore {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

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

    pub async fn upsert_klines(&self, klines: &[Kline]) -> Result<u64, StoreError> {
        if klines.is_empty() {
            return Ok(0);
        }

        let mut builder = QueryBuilder::<Postgres>::new(
            r#"
            INSERT INTO klines (
                source, symbol, interval, open_time, open_price, high_price,
                low_price, close_price, base_volume, quote_volume,
                source_count, is_complete
            )
            "#,
        );

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
        Ok(result.rows_affected())
    }
}
