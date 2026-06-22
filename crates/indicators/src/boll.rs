//! BOLL（布林带）与 %B 指标计算。
//!
//! - mid = SMA(close, period)
//! - std = population_standard_deviation(close, period)
//! - upper = mid + multiplier * std
//! - lower = mid - multiplier * std
//! - bandwidth = (upper - lower) / |mid|
//! - percent_b = (close - lower) / (upper - lower)

use crate::types::{sma, rolling_std, IndicatorValue};

pub struct BollResult {
    pub upper: Vec<IndicatorValue>,
    pub mid: Vec<IndicatorValue>,
    pub lower: Vec<IndicatorValue>,
    pub bandwidth: Vec<IndicatorValue>,
    pub percent_b: Vec<IndicatorValue>,
}

/// 计算 BOLL 指标。
/// - `close`：收盘价序列
/// - `period`：默认 20
/// - `multiplier`：默认 2.0
pub fn compute_boll(close: &[f64], period: usize, multiplier: f64) -> BollResult {
    let mid = sma(close, period);
    let std = rolling_std(close, period);

    let n = close.len();
    let mut upper = Vec::with_capacity(n);
    let mut lower = Vec::with_capacity(n);
    let mut bandwidth = Vec::with_capacity(n);
    let mut percent_b = Vec::with_capacity(n);

    for i in 0..n {
        if !crate::types::is_finite(close[i]) {
            upper.push(IndicatorValue::Unavailable("non-finite".into()));
            lower.push(IndicatorValue::Unavailable("non-finite".into()));
            bandwidth.push(IndicatorValue::Unavailable("non-finite".into()));
            percent_b.push(IndicatorValue::Unavailable("non-finite".into()));
            continue;
        }

        match (mid[i].value(), std[i].value()) {
            (Some(m), Some(s)) => {
                let up = m + multiplier * s;
                let lo = m - multiplier * s;
                upper.push(IndicatorValue::Available(up));
                lower.push(IndicatorValue::Available(lo));

                // bandwidth
                if m.abs() > f64::EPSILON {
                    bandwidth.push(IndicatorValue::Available((up - lo) / m.abs()));
                } else {
                    bandwidth.push(IndicatorValue::Unavailable("mid is zero".into()));
                }

                // percent_b
                let denom = up - lo;
                if denom > f64::EPSILON {
                    percent_b.push(IndicatorValue::Available((close[i] - lo) / denom));
                } else {
                    percent_b.push(IndicatorValue::Unavailable("upper == lower".into()));
                }
            }
            _ => {
                upper.push(IndicatorValue::Unavailable("BOLL unavailable".into()));
                lower.push(IndicatorValue::Unavailable("BOLL unavailable".into()));
                bandwidth.push(IndicatorValue::Unavailable("BOLL unavailable".into()));
                percent_b.push(IndicatorValue::Unavailable("BOLL unavailable".into()));
            }
        }
    }

    BollResult {
        upper,
        mid,
        lower,
        bandwidth,
        percent_b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 使用 TA-Lib 官方文档的验证数据：20 个等值 close
    #[test]
    fn test_boll_constant_prices() {
        let close = vec![100.0; 30];
        let result = compute_boll(&close, 20, 2.0);
        // index 19+: bandwidth should be 0, %B should be unavailable
        assert!(result.mid[19].value().is_some());
        assert!((result.mid[19].value().unwrap() - 100.0).abs() < 0.001);
        assert!(result.upper[19].value().unwrap() - 100.0 < 0.001);
        assert!(result.lower[19].value().unwrap() - 100.0 < 0.001);
        // bandwidth = (100-100)/100 = 0
        assert!((result.bandwidth[19].value().unwrap() - 0.0).abs() < 0.001);
        // %B unavailable because upper == lower
        assert!(!result.percent_b[19].is_available());
    }

    #[test]
    fn test_boll_varying_prices() {
        let close: Vec<f64> = (1..=50).map(|x| 100.0 + x as f64).collect();
        let result = compute_boll(&close, 20, 2.0);
        // After 20 bars, upper > mid > lower
        assert!(result.upper[25].value().unwrap() > result.mid[25].value().unwrap());
        assert!(result.mid[25].value().unwrap() > result.lower[25].value().unwrap());
        // %B should be between 0 and 1 for data within the band
        let b = result.percent_b[25].value().unwrap();
        assert!(b >= 0.0 && b <= 1.0, "percent_b={b} should be in [0,1]");
        // bandwidth > 0
        assert!(result.bandwidth[25].value().unwrap() > 0.0);
    }
}
