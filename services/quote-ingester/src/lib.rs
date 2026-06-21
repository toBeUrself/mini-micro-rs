//! quote-ingester crate 的库入口。
//!
//! Rust 里一个 crate 可以同时有 `lib.rs` 和 `main.rs`：
//! - `main.rs` 负责启动进程。
//! - `lib.rs` 暴露可复用的模块、类型和函数，方便测试或被别的 crate 调用。

/// K 线聚合逻辑：把 1m 聚合成 5m、30m 等更大周期。
pub mod aggregate;
/// Binance 行情 HTTP 客户端和响应解析。
pub mod api;
/// TOML 配置解析、默认值和配置校验。
pub mod config;
/// 服务内部使用的数据模型。
pub mod models;
/// Postgres 读写逻辑。
pub mod store;
/// 常驻 worker：负责回填、轮询、写库和触发聚合。
pub mod worker;

/// 对外暴露配置类型，main.rs 会直接使用。
pub use config::{QuoteIngesterConfig, ResolvedConfig};
/// 对外暴露 Postgres 存储实现，main.rs 需要用它先跑 migration。
pub use store::PostgresKlineStore;
/// 对外暴露 worker 入口。
pub use worker::run;
