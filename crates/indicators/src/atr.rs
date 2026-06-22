//! ATR（平均真实波幅）指标计算。
//!
//! - TR = max(high - low, |high - prev_close|, |low - prev_close|)
//! - 首个 ATR = 前 period 个 TR 的 SMA
//! - 后续 ATR = (prev_ATR * (period - 1) + TR_now) / period

use crate::types::IndicatorValue;

pub struct AtrResult {
    pub tr: Vec<IndicatorValue>,
    pub atr: Vec<IndicatorValue>,
}

/// 计算 ATR 指标。
/// - `high`、`low`、`close`：OHLC 序列
/// - `period`：默认 14
pub fn compute_atr(high: &[f64], low: &[f64], close: &[f64], period: usize) -> AtrResult {
    let n = high.len().min(low.len()).min(close.len());
    let mut tr = Vec::with_capacity(n);
    let mut atr = Vec::with_capacity(n);

    for i in 0..n {
        if !crate::types::is_finite(high[i])
            || !crate::types::is_finite(low[i])
            || !crate::types::is_finite(close[i])
        {
            tr.push(IndicatorValue::Unavailable("non-finite".into()));
            atr.push(IndicatorValue::Unavailable("non-finite".into()));
            continue;
        }

        let prev_close = if i == 0 { close[0] } else { close[i - 1] };

        let true_range = (high[i] - low[i])
            .max((high[i] - prev_close).abs())
            .max((low[i] - prev_close).abs());

        tr.push(IndicatorValue::Available(true_range));
    }

    // 计算 ATR
    for i in 0..n {
        if i < period - 1 {
            atr.push(IndicatorValue::Unavailable(format!(
                "need {period} bars, have {}",
                i + 1
            )));
        } else if i == period - 1 {
            // 首个 ATR = SMA of first `period` TR values
            let sum: f64 = tr[..=i]
                .iter()
                .filter_map(|t| t.value())
                .sum();
            atr.push(IndicatorValue::Available(sum / period as f64));
        } else {
            match (atr[i - 1].value(), tr[i].value()) {
                (Some(prev_atr), Some(tr_now)) => {
                    // Wilder smoothing
                    let val = (prev_atr * (period - 1) as f64 + tr_now) / period as f64;
                    atr.push(IndicatorValue::Available(val));
                }
                _ => atr.push(IndicatorValue::Unavailable("prev ATR unavailable".into())),
            }
        }
    }

    AtrResult { tr, atr }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atr_basic() {
        let high: Vec<f64> = vec![10.0, 12.0, 13.0, 11.0, 14.0, 15.0, 16.0, 14.0, 13.0, 15.0, 16.0, 18.0, 20.0, 19.0, 18.0, 17.0, 16.0, 19.0, 22.0, 21.0];
        let low: Vec<f64> = vec![8.0, 9.0, 10.0, 8.0, 11.0, 12.0, 13.0, 10.0, 9.0, 12.0, 13.0, 15.0, 17.0, 16.0, 14.0, 13.0, 12.0, 15.0, 18.0, 17.0];
        let close: Vec<f64> = vec![9.0, 11.0, 12.0, 9.0, 13.0, 14.0, 15.0, 12.0, 11.0, 14.0, 15.0, 17.0, 19.0, 17.0, 16.0, 15.0, 14.0, 17.0, 20.0, 19.0];

        let result = compute_atr(&high, &low, &close, 14);
        // TR should exist for all bars
        assert!(result.tr[0].is_available());
        // ATR needs 14 bars
        assert!(!result.atr[12].is_available());
        assert!(result.atr[13].value().is_some());
    }
}
