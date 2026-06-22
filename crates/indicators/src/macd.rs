//! MACD 指标计算。
//!
//! - dif = EMA(close, fast) - EMA(close, slow)
//! - dea = EMA(dif, signal)
//! - hist = dif - dea
//! - 金叉：hist_prev <= 0 && hist_now > 0
//! - 死叉：hist_prev >= 0 && hist_now < 0

use crate::types::{ema, IndicatorValue};

pub struct MacdResult {
    pub dif: Vec<IndicatorValue>,
    pub dea: Vec<IndicatorValue>,
    pub hist: Vec<IndicatorValue>,
    /// 金叉信号（每个位置是否有金叉）。
    pub golden_cross: Vec<bool>,
    /// 死叉信号（每个位置是否有死叉）。
    pub death_cross: Vec<bool>,
}

/// 计算 MACD 指标。
/// 默认参数：fast=12, slow=26, signal=9。
pub fn compute_macd(close: &[f64], fast: usize, slow: usize, signal: usize) -> MacdResult {
    let ema_fast = ema(close, fast);
    let ema_slow = ema(close, slow);

    let n = close.len();
    let mut dif = Vec::with_capacity(n);
    let mut hist = Vec::with_capacity(n);
    let mut golden_cross = Vec::with_capacity(n);
    let mut death_cross = Vec::with_capacity(n);

    for i in 0..n {
        // DIF
        match (ema_fast[i].value(), ema_slow[i].value()) {
            (Some(f), Some(s)) => {
                dif.push(IndicatorValue::Available(f - s));
            }
            _ => {
                dif.push(IndicatorValue::Unavailable("DIF unavailable".into()));
            }
        }
    }

    // DEA = EMA of DIF
    // Only compute DEA from first available DIF; pad beginning with Unavailable.
    let dif_values: Vec<f64> = dif
        .iter()
        .map(|d| d.value().unwrap_or(f64::NAN))
        .collect();

    // Find first available DIF index
    let first_available = dif.iter().position(|d| d.is_available()).unwrap_or(n);
    let dea = if first_available < n {
        let valid_dif = &dif_values[first_available..];
        let mut dea_tail = ema(valid_dif, signal);
        let mut dea_full: Vec<IndicatorValue> = (0..first_available)
            .map(|_| IndicatorValue::Unavailable("DIF unavailable".into()))
            .collect();
        dea_full.append(&mut dea_tail);
        // Ensure length matches `n`
        dea_full.resize(n, IndicatorValue::Unavailable("DIF unavailable".into()));
        dea_full
    } else {
        vec![IndicatorValue::Unavailable("no DIF available".into()); n]
    };

    // hist = DIF - DEA
    for i in 0..n {
        match (dif[i].value(), dea[i].value()) {
            (Some(d), Some(e)) => {
                let h = d - e;
                hist.push(IndicatorValue::Available(h));
            }
            _ => {
                hist.push(IndicatorValue::Unavailable("hist unavailable".into()));
            }
        }
    }

    // 金叉/死叉
    for i in 0..n {
        if i == 0 {
            golden_cross.push(false);
            death_cross.push(false);
        } else {
            let prev_hist = hist[i - 1].value();
            let curr_hist = hist[i].value();
            match (prev_hist, curr_hist) {
                (Some(p), Some(c)) => {
                    golden_cross.push(p <= 0.0 && c > 0.0);
                    death_cross.push(p >= 0.0 && c < 0.0);
                }
                _ => {
                    golden_cross.push(false);
                    death_cross.push(false);
                }
            }
        }
    }

    MacdResult {
        dif,
        dea,
        hist,
        golden_cross,
        death_cross,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macd_warmup() {
        let close: Vec<f64> = (1..=100).map(|x| 100.0 + x as f64 * 0.5).collect();
        let result = compute_macd(&close, 12, 26, 9);
        // fast(12): first available after 11 bars (index 11)
        // slow(26): first available after 25 bars (index 25)
        // DIF = fast - slow, first available at max(11, 25) = 25
        assert!(!result.dif[24].is_available());
        assert!(result.dif[25].is_available());
        // DEA = EMA of DIF with signal period=9.
        // DIF first available at index 25. DEA starts from index 25.
        // EMA period=9: seed needs 9 bars. First available: 25+9-1=33.
        assert!(result.dea[33].is_available());
        assert!(result.dea[34].is_available());
    }

    #[test]
    fn test_macd_golden_cross() {
        // Need enough bars for MACD warmup. Use 200 bars.
        let mut close: Vec<f64> = vec![100.0; 200];
        // Sharp rise at end: fast EMA will cross above slow EMA
        for i in 150..200 {
            close[i] = 100.0 + (i - 150) as f64 * 5.0;
        }
        let result = compute_macd(&close, 12, 26, 9);
        assert!(result.dif.last().unwrap().is_available());
        assert!(result.dea.last().unwrap().is_available());
        let last_hist = result.hist.last().unwrap();
        assert!(last_hist.is_available());
        // With sharp rise, DIF should be positive (fast EMA > slow EMA)
        assert!(result.dif.last().unwrap().value().unwrap() > 0.0);
    }

    #[test]
    fn test_macd_death_cross() {
        // Need at least slow(26)+signal(9) ≈ 35 bars for MACD to be available.
        // Use 200 bars: first 150 flat at 150.0, last 50 decline sharply.
        let mut close: Vec<f64> = vec![150.0; 200];
        for i in 150..200 {
            close[i] = 150.0 - (i - 150) as f64 * 3.0;
        }
        let result = compute_macd(&close, 12, 26, 9);
        assert!(result.dif.last().unwrap().is_available());
        assert!(result.dea.last().unwrap().is_available());
        let last_hist = result.hist.last().unwrap();
        assert!(last_hist.is_available());
        // With a sharp decline, hist should be negative
        assert!(last_hist.value().unwrap() < 0.0);
    }
}
