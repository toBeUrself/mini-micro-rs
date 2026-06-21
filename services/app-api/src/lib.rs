//! `app-api` 是业务 API 服务。
//!
//! 它和 `gateway`、`quote-ingester` 的职责不同：
//! - `gateway`：统一入口，做登录鉴权、请求转发。
//! - `quote-ingester`：后台采集行情数据并写入数据库。
//! - `app-api`：读取数据库，提供业务查询接口。
//!
//! 后面如果要提供 users、订单、分析结果等接口，也会继续放在这个 crate 里。

pub mod app;
pub mod config;
pub mod error;
pub mod market;
pub mod store;

pub use app::{router, AppState};
pub use config::{AppApiConfig, ResolvedConfig};
pub use store::PostgresAppStore;
