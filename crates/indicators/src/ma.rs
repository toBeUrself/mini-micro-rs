//! MA / EMA 指标计算。
//!
//! - MA20 / MA60：简单移动平均
//! - EMA20：指数移动平均
//! - ma_spread：(max(MA20, MA60) - min(MA20, MA60)) / close
//! - ma20_slope：(MA20_now - MA20_n_bars_ago) / MA20_n_bars_ago
//! - ema20_deviation：(close - EMA20) / EMA20

use crate::types::{sma, IndicatorValue};

/// 计算结果。
pub struct MaResult {
    pub ma20: Vec<IndicatorValue>,
    pub ma60: Vec<IndicatorValue>,
    pub ema20: Vec<IndicatorValue>,
    pub ma_spread: Vec<IndicatorValue>,
    pub ma20_slope: Vec<IndicatorValue>,
    pub ema20_deviation: Vec<IndicatorValue>,
}

/// 计算 MA/EMA 指标。
///
/// - `close`：收盘价序列
/// - `slope_lookback`：MA20 斜率回看 bar 数，默认 5
pub fn compute_ma(close: &[f64], slope_lookback: usize) -> MaResult {
    let ma20 = sma(close, 20);
    let ma60 = sma(close, 60);
    let ema20 = crate::types::ema(close, 20);

    let mut ma_spread = Vec::with_capacity(close.len());
    let mut ma20_slope = Vec::with_capacity(close.len());
    let mut ema20_deviation = Vec::with_capacity(close.len());

    for i in 0..close.len() {
        if !crate::types::is_finite(close[i]) {
            ma_spread.push(IndicatorValue::Unavailable("non-finite close".into()));
            ma20_slope.push(IndicatorValue::Unavailable("non-finite close".into()));
            ema20_deviation.push(IndicatorValue::Unavailable("non-finite close".into()));
            continue;
        }

        // ma_spread
        match (ma20[i].value(), ma60[i].value()) {
            (Some(m20), Some(m60)) => {
                let spread = if close[i].abs() > f64::EPSILON {
                    (m20.max(m60) - m20.min(m60)) / close[i].abs()
                } else {
                    0.0
                };
                ma_spread.push(IndicatorValue::Available(spread));
            }
            _ => {
                ma_spread.push(IndicatorValue::Unavailable("MA unavailable".into()));
            }
        }

        // ma20_slope
        if i >= slope_lookback {
            match (ma20[i].value(), ma20[i - slope_lookback].value()) {
                (Some(curr), Some(prev)) => {
                    if prev.abs() > f64::EPSILON {
                        let slope = (curr - prev) / prev.abs();
                        ma20_slope.push(IndicatorValue::Available(slope));
                    } else {
                        ma20_slope.push(IndicatorValue::Available(0.0));
                    }
                }
                _ => ma20_slope.push(IndicatorValue::Unavailable("MA20 slope unavailable".into())),
            }
        } else {
            ma20_slope.push(IndicatorValue::Unavailable("insufficient bars".into()));
        }

        // ema20_deviation
        match ema20[i].value() {
            Some(e20) => {
                if e20.abs() > f64::EPSILON {
                    let dev = (close[i] - e20) / e20.abs();
                    ema20_deviation.push(IndicatorValue::Available(dev));
                } else {
                    ema20_deviation.push(IndicatorValue::Available(0.0));
                }
            }
            None => ema20_deviation.push(IndicatorValue::Unavailable("EMA20 unavailable".into())),
        }
    }

    MaResult {
        ma20,
        ma60,
        ema20,
        ma_spread,
        ma20_slope,
        ema20_deviation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_close(n: usize) -> Vec<f64> {
        (1..=n).map(|x| 100.0 + x as f64 * 0.5).collect()
    }

    #[test]
    fn test_ma20_requires_20_bars() {
        let close = make_close(100);
        let result = compute_ma(&close, 5);
        assert!(result.ma20[18].value().is_none());
        assert!(result.ma20[19].value().is_some());
    }

    #[test]
    fn test_ma60_requires_60_bars() {
        let close = make_close(100);
        let result = compute_ma(&close, 5);
        assert!(result.ma60[58].value().is_none());
        assert!(result.ma60[59].value().is_some());
    }

    #[test]
    fn test_ema20_deviation_sign() {
        let mut close = make_close(100);
        // make last close significantly above EMA20
        close[99] = 1000.0;
        let result = compute_ma(&close, 5);
        assert!(result.ema20_deviation[99].value().unwrap() > 0.0);
    }

    #[test]
    fn test_ma20_slope_requires_lookback() {
        let close = make_close(100);
        let result = compute_ma(&close, 5);
        // slope needs MA20 available at i and i-5, so first available index is 24
        assert!(result.ma20_slope[23].value().is_none());
        // index 24: MA20[24] and MA20[19] should both be available
        assert!(result.ma20_slope[24].value().is_some());
    }
}
