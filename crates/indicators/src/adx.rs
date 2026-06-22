//! ADX / DMI 指标计算。
//!
//! - +DM = max(0, high - prev_high) if high - prev_high > prev_low - low
//! - -DM = max(0, prev_low - low) if prev_low - low > high - prev_high
//! - +DI = 100 * EMA(+DM, period) / ATR
//! - -DI = 100 * EMA(-DM, period) / ATR
//! - DX = 100 * |+DI - -DI| / (+DI + -DI)
//! - ADX = EMA(DX, period)

use crate::types::{ema, IndicatorValue};

pub struct AdxResult {
    pub adx: Vec<IndicatorValue>,
    pub plus_di: Vec<IndicatorValue>,
    pub minus_di: Vec<IndicatorValue>,
    pub dx: Vec<IndicatorValue>,
}

/// 计算 ADX/DMI 指标。
/// - `high`、`low`、`close`：OHLC 序列
/// - `period`：默认 14
/// - 需要外部传入已计算好的 ATR 序列
pub fn compute_adx(
    high: &[f64],
    low: &[f64],
    close: &[f64],
    atr: &[IndicatorValue],
    period: usize,
) -> AdxResult {
    let n = high.len().min(low.len()).min(close.len()).min(atr.len());

    // 计算 +DM 和 -DM
    let mut plus_dm_raw = Vec::with_capacity(n);
    let mut minus_dm_raw = Vec::with_capacity(n);

    for i in 0..n {
        if i == 0 {
            plus_dm_raw.push(0.0);
            minus_dm_raw.push(0.0);
            continue;
        }
        if !crate::types::is_finite(high[i])
            || !crate::types::is_finite(low[i])
            || !crate::types::is_finite(high[i - 1])
            || !crate::types::is_finite(low[i - 1])
        {
            plus_dm_raw.push(0.0);
            minus_dm_raw.push(0.0);
            continue;
        }

        let up_move = high[i] - high[i - 1];
        let down_move = low[i - 1] - low[i];

        if up_move > down_move && up_move > 0.0 {
            plus_dm_raw.push(up_move);
        } else {
            plus_dm_raw.push(0.0);
        }

        if down_move > up_move && down_move > 0.0 {
            minus_dm_raw.push(down_move);
        } else {
            minus_dm_raw.push(0.0);
        }
    }

    // EMA of +DM and -DM
    let plus_di_raw = ema(&plus_dm_raw, period);
    let minus_di_raw = ema(&minus_dm_raw, period);

    let mut plus_di = Vec::with_capacity(n);
    let mut minus_di = Vec::with_capacity(n);
    let mut dx = Vec::with_capacity(n);

    for i in 0..n {
        match (plus_di_raw[i].value(), minus_di_raw[i].value(), atr[i].value()) {
            (Some(pdi), Some(mdi), Some(atr_val)) if atr_val > f64::EPSILON => {
                let pdi_norm = 100.0 * pdi / atr_val;
                let mdi_norm = 100.0 * mdi / atr_val;
                plus_di.push(IndicatorValue::Available(pdi_norm));
                minus_di.push(IndicatorValue::Available(mdi_norm));

                let denom = pdi_norm + mdi_norm;
                if denom > f64::EPSILON {
                    let dx_val = 100.0 * (pdi_norm - mdi_norm).abs() / denom;
                    dx.push(IndicatorValue::Available(dx_val));
                } else {
                    dx.push(IndicatorValue::Unavailable("DI sum zero".into()));
                }
            }
            _ => {
                plus_di.push(IndicatorValue::Unavailable("DI unavailable".into()));
                minus_di.push(IndicatorValue::Unavailable("DI unavailable".into()));
                dx.push(IndicatorValue::Unavailable("DI unavailable".into()));
            }
        }
    }

    // ADX = EMA of DX
    let dx_values: Vec<f64> = dx.iter().map(|d| d.value().unwrap_or(f64::NAN)).collect();
    let adx = ema(&dx_values, period);

    AdxResult {
        adx,
        plus_di,
        minus_di,
        dx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atr::compute_atr;

    #[test]
    fn test_adx_warmup() {
        let n = 100;
        let high: Vec<f64> = (0..n).map(|i| 100.0 + i as f64 * 0.5).collect();
        let low: Vec<f64> = (0..n).map(|i| 99.0 + i as f64 * 0.5).collect();
        let close: Vec<f64> = (0..n).map(|i| 99.5 + i as f64 * 0.5).collect();
        let atr_result = compute_atr(&high, &low, &close, 14);
        let result = compute_adx(&high, &low, &close, &atr_result.atr, 14);

        // ADX won't be available until ATR + DM EMAs are ready
        let last_adx = result.adx.last().unwrap();
        assert!(last_adx.is_available());
    }
}
