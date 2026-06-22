//! Volume Ratio 指标计算。
//!
//! - volume_ratio = current_volume / SMA(volume, period)
//! - volume_ratio >= 1.5：放量
//! - 0.7 <= volume_ratio <= 1.3：成交量平稳
//! - volume_ratio < 0.7：缩量

use crate::types::{sma, IndicatorValue};

/// 计算 Volume Ratio。
/// - `volume`：成交量序列
/// - `period`：SMA 周期，默认 20
pub fn compute_volume_ratio(volume: &[f64], period: usize) -> Vec<IndicatorValue> {
    let vol_ma = sma(volume, period);
    let n = volume.len();
    let mut result = Vec::with_capacity(n);

    for i in 0..n {
        if !crate::types::is_finite(volume[i]) {
            result.push(IndicatorValue::Unavailable("non-finite volume".into()));
            continue;
        }
        match vol_ma[i].value() {
            Some(ma) => {
                if ma > f64::EPSILON {
                    result.push(IndicatorValue::Available(volume[i] / ma));
                } else {
                    result.push(IndicatorValue::Unavailable("volume MA is zero".into()));
                }
            }
            None => result.push(IndicatorValue::Unavailable("volume MA unavailable".into())),
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_ratio() {
        let volume = vec![100.0; 50];
        let result = compute_volume_ratio(&volume, 20);
        assert!(!result[18].is_available());
        assert!(result[19].is_available());
        assert!((result[19].value().unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_surge_volume() {
        let mut volume = vec![100.0; 50];
        volume[49] = 200.0;
        let result = compute_volume_ratio(&volume, 20);
        assert!(result[49].value().unwrap() > 1.5);
    }
}
