//! RSI（相对强弱指标）计算。
//!
//! 使用 Wilder 平滑：
//! - 首个 avg_gain = 前 period 个 gain 的 SMA
//! - 后续 avg_gain = (prev_avg_gain * (period-1) + gain_now) / period
//! - RS = avg_gain / avg_loss
//! - RSI = 100 - 100/(1 + RS)

use crate::types::IndicatorValue;

/// 计算 RSI 指标。
/// - `close`：收盘价序列
/// - `period`：默认 14
pub fn compute_rsi(close: &[f64], period: usize) -> Vec<IndicatorValue> {
    let n = close.len();
    let mut rsi = Vec::with_capacity(n);

    if period == 0 || n < period + 1 {
        for _ in 0..n {
            rsi.push(IndicatorValue::Unavailable("insufficient data".into()));
        }
        return rsi;
    }

    let mut gains = Vec::with_capacity(n);
    let mut losses = Vec::with_capacity(n);

    for i in 0..n {
        if i == 0 {
            gains.push(0.0_f64);
            losses.push(0.0_f64);
        } else if !crate::types::is_finite(close[i]) || !crate::types::is_finite(close[i - 1]) {
            gains.push(f64::NAN);
            losses.push(f64::NAN);
        } else {
            let diff = close[i] - close[i - 1];
            if diff > 0.0 {
                gains.push(diff);
                losses.push(0.0);
            } else {
                gains.push(0.0);
                losses.push(-diff);
            }
        }
    }

    let mut avg_gain: f64 = 0.0;
    let mut avg_loss: f64 = 0.0;
    let mut initialized = false;

    for i in 0..n {
        if i < period {
            rsi.push(IndicatorValue::Unavailable(format!(
                "need {period}+1 bars, have {}",
                i + 1
            )));
            continue;
        }

        if !initialized {
            // First RSI: SMA of first `period` gains/losses (excluding index 0)
            let gain_sum: f64 = gains[1..=period].iter().filter(|g| !g.is_nan()).sum();
            let loss_sum: f64 = losses[1..=period].iter().filter(|l| !l.is_nan()).sum();
            avg_gain = gain_sum / period as f64;
            avg_loss = loss_sum / period as f64;
            initialized = true;
        } else {
            if !gains[i].is_nan() && !losses[i].is_nan() {
                // Wilder smoothing
                avg_gain = (avg_gain * (period - 1) as f64 + gains[i]) / period as f64;
                avg_loss = (avg_loss * (period - 1) as f64 + losses[i]) / period as f64;
            }
        }

        if avg_loss < f64::EPSILON {
            if avg_gain > f64::EPSILON {
                rsi.push(IndicatorValue::Available(100.0));
            } else {
                rsi.push(IndicatorValue::Unavailable("no movement".into()));
            }
        } else {
            let rs = avg_gain / avg_loss;
            let val = 100.0 - 100.0 / (1.0 + rs);
            rsi.push(IndicatorValue::Available(val.clamp(0.0, 100.0)));
        }
    }

    rsi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rsi_warmup() {
        let close: Vec<f64> = (0..=20).map(|i| 100.0 + i as f64 * 0.5).collect();
        let result = compute_rsi(&close, 14);
        // Need period+1 bars, so index 14 is the first available
        assert!(!result[13].is_available());
        assert!(result[14].is_available());
        // All up moves → RSI should be 100
        assert!((result[14].value().unwrap() - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_rsi_mixed() {
        let close = vec![
            100.0, 102.0, 101.0, 103.0, 104.0, 102.0, 100.0, 101.0, 103.0, 105.0,
            104.0, 106.0, 108.0, 107.0, 109.0, 110.0, 108.0, 107.0, 106.0, 105.0,
        ];
        let result = compute_rsi(&close, 14);
        assert!(result[14].is_available());
        let val = result[14].value().unwrap();
        assert!(val > 0.0 && val < 100.0);
    }
}
