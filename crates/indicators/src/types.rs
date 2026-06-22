//! 指标计算公共类型。

/// 指标计算结果状态：可用或不可用。
#[derive(Debug, Clone, PartialEq)]
pub enum IndicatorValue {
    /// 指标计算成功，携带值。
    Available(f64),
    /// 指标不可用，携带原因。
    Unavailable(String),
}

impl IndicatorValue {
    /// 如果可用则返回值，否则返回 None。
    pub fn value(&self) -> Option<f64> {
        match self {
            Self::Available(v) => Some(*v),
            Self::Unavailable(_) => None,
        }
    }

    /// 是否可用。
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }
}

/// 一组指标的可用性汇总。
#[derive(Debug, Clone)]
pub struct IndicatorAvailability {
    /// 所有核心指标是否就绪。
    pub ready: bool,
    /// 最小必需的 K 线数量。
    pub min_required_bars: usize,
    /// 建议的 warmup K 线数量。
    pub warmup_bars: usize,
    /// 当前不可用的指标字段名列表。
    pub unavailable_fields: Vec<String>,
}

impl IndicatorAvailability {
    pub fn new(min_required_bars: usize, warmup_bars: usize) -> Self {
        Self {
            ready: false,
            min_required_bars,
            warmup_bars,
            unavailable_fields: Vec::new(),
        }
    }
}

/// 线性衰减：x <= x_low → 100，x >= x_high → 0，中间线性过渡。
/// 输出 clamp 到 0~100。
pub fn linear_down(x: f64, x_low: f64, x_high: f64) -> f64 {
    if x <= x_low {
        return 100.0;
    }
    if x >= x_high {
        return 0.0;
    }
    ((x_high - x) / (x_high - x_low) * 100.0).clamp(0.0, 100.0)
}

/// 线性上升：x <= x_low → 0，x >= x_high → 100，中间线性过渡。
/// 输出 clamp 到 0~100。
pub fn linear_up(x: f64, x_low: f64, x_high: f64) -> f64 {
    if x <= x_low {
        return 0.0;
    }
    if x >= x_high {
        return 100.0;
    }
    ((x - x_low) / (x_high - x_low) * 100.0).clamp(0.0, 100.0)
}

/// 检查值是否有限（不是 NaN 也不是 Inf）。
pub fn is_finite(x: f64) -> bool {
    x.is_finite()
}

/// SMA（简单移动平均）。
/// 不足 period 时返回可用数量的 SMA；若无数据则返回空 vec。
pub fn sma(values: &[f64], period: usize) -> Vec<IndicatorValue> {
    if period == 0 || values.is_empty() {
        return values.iter().map(|_| IndicatorValue::Unavailable("no data".into())).collect();
    }

    let mut result = Vec::with_capacity(values.len());
    let mut sum: f64 = 0.0;
    let mut count: usize = 0;

    for (i, &v) in values.iter().enumerate() {
        if !is_finite(v) {
            result.push(IndicatorValue::Unavailable(format!("non-finite at index {i}")));
            continue;
        }
        sum += v;
        count += 1;
        if count > period {
            // 减去窗口外的值
            sum -= values[i - period];
            count = period;
        }
        if count < period {
            result.push(IndicatorValue::Unavailable(format!(
                "need {period} bars, have {count}"
            )));
        } else {
            result.push(IndicatorValue::Available(sum / count as f64));
        }
    }

    result
}

/// EMA（指数移动平均）。
/// 首期 EMA 使用第一个 period 的 SMA 作为种子。
pub fn ema(values: &[f64], period: usize) -> Vec<IndicatorValue> {
    if period == 0 || values.is_empty() {
        return values.iter().map(|_| IndicatorValue::Unavailable("no data".into())).collect();
    }

    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut result = Vec::with_capacity(values.len());

    for (i, &v) in values.iter().enumerate() {
        if !is_finite(v) {
            result.push(IndicatorValue::Unavailable(format!("non-finite at index {i}")));
            continue;
        }
        if i < period - 1 {
            result.push(IndicatorValue::Unavailable(format!(
                "need {period} bars for seed, have {}",
                i + 1
            )));
        } else if i == period - 1 {
            // 种子：前 period 个的 SMA
            let seed_sum: f64 = values[..=i].iter().sum();
            result.push(IndicatorValue::Available(seed_sum / period as f64));
        } else {
            match &result[i - 1] {
                IndicatorValue::Available(prev) => {
                    let val = (v - prev) * multiplier + prev;
                    result.push(IndicatorValue::Available(val));
                }
                IndicatorValue::Unavailable(_) => {
                    result.push(IndicatorValue::Unavailable("previous EMA unavailable".into()));
                }
            }
        }
    }

    result
}

/// 总体标准差（population standard deviation）。
pub fn std_dev(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    if !is_finite(variance) {
        return None;
    }
    Some(variance.sqrt())
}

/// 滚动标准差。
pub fn rolling_std(values: &[f64], period: usize) -> Vec<IndicatorValue> {
    if period == 0 || values.is_empty() {
        return values.iter().map(|_| IndicatorValue::Unavailable("no data".into())).collect();
    }

    let mut result = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        if i + 1 < period {
            result.push(IndicatorValue::Unavailable(format!(
                "need {period} bars, have {}",
                i + 1
            )));
        } else {
            let window = &values[i + 1 - period..=i];
            match std_dev(window) {
                Some(s) => result.push(IndicatorValue::Available(s)),
                None => result.push(IndicatorValue::Unavailable("std_dev failed".into())),
            }
        }
    }
    result
}

/// 滚动窗口最大值。
pub fn rolling_max(values: &[f64], period: usize) -> Vec<IndicatorValue> {
    if period == 0 || values.is_empty() {
        return values.iter().map(|_| IndicatorValue::Unavailable("no data".into())).collect();
    }

    let mut result = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        if i + 1 < period {
            result.push(IndicatorValue::Unavailable(format!(
                "need {period} bars, have {}",
                i + 1
            )));
        } else {
            let window = &values[i + 1 - period..=i];
            let max = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            result.push(IndicatorValue::Available(max));
        }
    }
    result
}

/// 滚动窗口最小值。
pub fn rolling_min(values: &[f64], period: usize) -> Vec<IndicatorValue> {
    if period == 0 || values.is_empty() {
        return values.iter().map(|_| IndicatorValue::Unavailable("no data".into())).collect();
    }

    let mut result = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        if i + 1 < period {
            result.push(IndicatorValue::Unavailable(format!(
                "need {period} bars, have {}",
                i + 1
            )));
        } else {
            let window = &values[i + 1 - period..=i];
            let min = window.iter().cloned().fold(f64::INFINITY, f64::min);
            result.push(IndicatorValue::Available(min));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_down() {
        assert!((linear_down(5.0, 10.0, 20.0) - 100.0).abs() < 0.001);
        assert!((linear_down(15.0, 10.0, 20.0) - 50.0).abs() < 0.001);
        assert!((linear_down(25.0, 10.0, 20.0) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_linear_up() {
        assert!((linear_up(5.0, 10.0, 20.0) - 0.0).abs() < 0.001);
        assert!((linear_up(15.0, 10.0, 20.0) - 50.0).abs() < 0.001);
        assert!((linear_up(25.0, 10.0, 20.0) - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_sma_basic() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = sma(&values, 3);
        assert!(result[0].value().is_none());
        assert!(result[1].value().is_none());
        assert!((result[2].value().unwrap() - 2.0).abs() < 0.001);
        assert!((result[3].value().unwrap() - 3.0).abs() < 0.001);
        assert!((result[4].value().unwrap() - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_ema_basic() {
        let values: Vec<f64> = (1..=10).map(|x| x as f64).collect();
        let result = ema(&values, 5);
        // index 4 is seed (SMA of first 5 = 3.0)
        assert!((result[4].value().unwrap() - 3.0).abs() < 0.001);
        // subsequent values
        assert!(result[5].is_available());
        assert!(result[9].is_available());
    }

    #[test]
    fn test_rolling_std() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = rolling_std(&values, 3);
        assert!(result[0].value().is_none());
        assert!(result[1].value().is_none());
        // population std of [1,2,3]: mean=2, var=(1+0+1)/3=2/3, std=sqrt(2/3)≈0.8165
        let expected = (2.0_f64 / 3.0).sqrt();
        assert!((result[2].value().unwrap() - expected).abs() < 0.001);
    }
}
