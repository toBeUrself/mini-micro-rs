use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::FromRow;

/// 服务内部统一使用的 K 线模型。
///
/// 这个结构体同时承担两个角色：
/// - 从接口响应转换后的内存数据。
/// - 从 Postgres `klines` 表查询出来的一行数据。
///
/// `FromRow` 是 sqlx 的派生宏，表示 SQL 查询结果可以自动映射成这个结构体。
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Kline {
    /// 数据来源，比如 `binance`。唯一键的一部分。
    pub source: String,
    /// 交易对，比如 `BTCUSDT`。唯一键的一部分。
    pub symbol: String,
    /// K 线周期，比如 `1m`、`5m`、`30m`。唯一键的一部分。
    pub interval: String,
    /// K 线开盘时间，统一保存成 UTC 时间。唯一键的一部分。
    pub open_time: DateTime<Utc>,
    /// 开盘价。
    pub open_price: Decimal,
    /// 最高价。
    pub high_price: Decimal,
    /// 最低价。
    pub low_price: Decimal,
    /// 收盘价。
    pub close_price: Decimal,
    /// 基础币成交量，比如 BTC 数量。
    pub base_volume: Decimal,
    /// 计价币成交额，比如 USDT 成交额。
    pub quote_volume: Decimal,
    /// 生成这根 K 线时使用了多少根源 K 线。
    ///
    /// 直接从交易所拉到的 1m K 线固定是 1。
    /// 本地聚合出来的 5m/30m 会记录实际参与聚合的 1m 数量。
    pub source_count: i32,
    /// 聚合窗口是否完整。
    ///
    /// 例如 5m 理论上需要 5 根 1m；如果只有 3 根，则这里是 false。
    pub is_complete: bool,
}

impl Kline {
    /// 构造一根 K 线。
    ///
    /// 参数比较多是因为 K 线本身字段多；这里保持显式传参，方便看清每个字段来自哪里。
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
            // `impl Into<String>` 允许调用方传 `String` 或 `&str`，这里统一转成 String 存储。
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
