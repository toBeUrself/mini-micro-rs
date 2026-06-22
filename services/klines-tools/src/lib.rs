//! klines-tools crate 库入口。
//!
//! 提供 K 线分析、指标计算、行情评分、状态机、网格计划、风险决策、前端展示数据、
//! 回测和复盘所需稳定契约。
//!
//! # 模块
//!
//! - `analyzer`：主分析编排器，串联所有模块
//! - `config`：TOML 配置解析、Feature Flag 管理
//! - `models`：所有数据模型
//! - `kline_reader`：从 app-api HTTP 接口拉取 K 线
//! - `data_validator`：OHLCV 校验、排序、去重、缺失检查、闭合识别
//! - `scoring`：三维评分系统
//! - `state_machine`：六状态状态机 + 假突破过滤
//! - `risk`：风险覆盖层与风险决策
//! - `confidence`：置信度分解
//! - `grid_plan`：网格计划生成 + GridLevel + 交易所约束
//! - `signal`：信号生成
//! - `multi_tf`：多周期时间对齐与合并决策
//! - `output`：JSON 输出契约构建

pub mod analyzer;
pub mod config;
pub mod confidence;
pub mod data_validator;
pub mod grid_plan;
pub mod kline_reader;
pub mod models;
pub mod multi_tf;
pub mod output;
pub mod risk;
pub mod scoring;
pub mod signal;
pub mod state_machine;

pub use analyzer::Analyzer;
pub use config::KlinesToolsConfig;
pub use models::*;
