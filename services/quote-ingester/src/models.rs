use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Kline {
    pub source: String,
    pub symbol: String,
    pub interval: String,
    pub open_time: DateTime<Utc>,
    pub open_price: Decimal,
    pub high_price: Decimal,
    pub low_price: Decimal,
    pub close_price: Decimal,
    pub base_volume: Decimal,
    pub quote_volume: Decimal,
    pub source_count: i32,
    pub is_complete: bool,
}

impl Kline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: impl Into<String>,
        symbol: impl Into<String>,
        interval: impl Into<String>,
        open_time: DateTime<Utc>,
        open_price: Decimal,
        high_price: Decimal,
        low_price: Decimal,
        close_price: Decimal,
        base_volume: Decimal,
        quote_volume: Decimal,
        source_count: i32,
        is_complete: bool,
    ) -> Self {
        Self {
            source: source.into(),
            symbol: symbol.into(),
            interval: interval.into(),
            open_time,
            open_price,
            high_price,
            low_price,
            close_price,
            base_volume,
            quote_volume,
            source_count,
            is_complete,
        }
    }
}
