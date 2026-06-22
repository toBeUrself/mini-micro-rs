//! Donchian Channel 指标计算。
//!
//! - donchian_high = highest(high, N)
//! - donchian_low = lowest(low, N)
//! - 突破判断建议使用上一根之前形成的通道边界，避免当前 K 线同时参与边界计算

use crate::types::IndicatorValue;

pub struct DonchianResult {
    pub upper: Vec<IndicatorValue>,
    pub lower: Vec<IndicatorValue>,
    pub mid: Vec<IndicatorValue>,
}

/// 计算 Donchian Channel。
/// - `high`、`low`：K 线序列
/// - `period`：默认 20
pub fn compute_donchian(high: &[f64], low: &[f64], period: usize) -> DonchianResult {
    let n = high.len().min(low.len());
    // 使用预先计算好的滚动最大/最小值
    let upper = crate::types::rolling_max(high, period);
    let lower = crate::types::rolling_min(low, period);

    let mut mid = Vec::with_capacity(n);
    for i in 0..n {
        match (upper[i].value(), lower[i].value()) {
            (Some(u), Some(l)) => {
                mid.push(IndicatorValue::Available((u + l) / 2.0));
            }
            _ => mid.push(IndicatorValue::Unavailable("Donchian unavailable".into())),
        }
    }

    DonchianResult { upper, lower, mid }
}

/// 检查向上突破（使用上上根形成的通道边界，避免未来函数）。
/// - `close`：当前收盘价
/// - `donchian_high_prev`：上一根之前的 Donchian 上轨
pub fn check_upper_breakout(close: f64, donchian_high_prev: f64) -> bool {
    close > donchian_high_prev
}

/// 检查向下破位。
pub fn check_lower_breakout(close: f64, donchian_low_prev: f64) -> bool {
    close < donchian_low_prev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_donchian_basic() {
        let high: Vec<f64> = (1..=30).map(|x| 100.0 + x as f64).collect();
        let low: Vec<f64> = (1..=30).map(|x| 90.0 + x as f64).collect();
        let result = compute_donchian(&high, &low, 20);
        assert!(!result.upper[18].is_available());
        assert!(result.upper[19].is_available());
        assert!(result.upper[19].value().unwrap() > result.lower[19].value().unwrap());
    }

    #[test]
    fn test_breakout_check() {
        // close above previous upper → breakout
        assert!(check_upper_breakout(150.0, 140.0));
        assert!(!check_upper_breakout(135.0, 140.0));
        assert!(check_lower_breakout(90.0, 100.0));
        assert!(!check_lower_breakout(105.0, 100.0));
    }
}
