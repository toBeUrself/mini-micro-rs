//! 纯指标计算库。
//!
//! 提供 K 线技术分析所需的各项指标计算，零 IO 依赖，纯数学运算。
//!
//! # 模块
//!
//! - `types`：公共类型（IndicatorValue、线性函数、SMA/EMA、滚动统计）
//! - `ma`：MA20/MA60/EMA20、均线粘合、斜率、偏离率
//! - `boll`：BOLL（布林带）、%B、带宽
//! - `macd`：MACD（DIF/DEA/histogram、金叉/死叉）
//! - `atr`：ATR（平均真实波幅）
//! - `adx`：ADX/DMI（趋势强度、多空方向）
//! - `rsi`：RSI（相对强弱）
//! - `vol_ratio`：Volume Ratio（成交量比率）
//! - `donchian`：Donchian Channel
//! - `price_structure`：价格结构（swing points、HH/HL/LH/LL）
//! - `percentile`：滚动分位数

pub mod adx;
pub mod atr;
pub mod boll;
pub mod donchian;
pub mod ma;
pub mod macd;
pub mod percentile;
pub mod price_structure;
pub mod rsi;
pub mod types;
pub mod vol_ratio;

// 常用 re-export
pub use types::{
    is_finite, linear_down, linear_up, sma, ema, std_dev, rolling_std, rolling_max, rolling_min,
    IndicatorAvailability, IndicatorValue,
};
