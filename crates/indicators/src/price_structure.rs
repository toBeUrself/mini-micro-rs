//! Price Structure 识别。
//!
//! - higher_high / higher_low / lower_high / lower_low
//! - swing_high / swing_low
//! - range_high / range_low

/// 价格结构识别结果。
#[derive(Debug, Clone, Default)]
pub struct PriceStructureResult {
    /// 每个位置是否为 swing_high。
    pub swing_high: Vec<bool>,
    /// 每个位置是否为 swing_low。
    pub swing_low: Vec<bool>,
    /// 最近是否形成 higher_high（基于最新 N 根 K 线的 swing points）。
    pub has_higher_high: bool,
    /// 最近是否形成 higher_low。
    pub has_higher_low: bool,
    /// 最近是否形成 lower_high。
    pub has_lower_high: bool,
    /// 最近是否形成 lower_low。
    pub has_lower_low: bool,
    /// 最近的震荡区间高位。
    pub range_high: Option<f64>,
    /// 最近的震荡区间低位。
    pub range_low: Option<f64>,
}

/// 识别价格结构。
///
/// - `high`、`low`：K 线序列
/// - `pivot_left`：确认 swing 所需的左侧 bar 数，默认 2
/// - `pivot_right`：确认 swing 所需的右侧 bar 数，默认 2
/// - `structure_lookback`：用于识别 HH/HL/LH/LL 的回看窗口
///
/// 为避免未来函数，实时模式下只有右侧 `pivot_right` 根 K 线闭合后，才能确认 swing 点。
pub fn identify_price_structure(
    high: &[f64],
    low: &[f64],
    pivot_left: usize,
    pivot_right: usize,
    structure_lookback: usize,
) -> PriceStructureResult {
    let n = high.len().min(low.len());
    let mut swing_high = vec![false; n];
    let mut swing_low = vec![false; n];

    // 识别 swing_high：当前 high 高于左右各 pivot_left/right 根 K 线 high
    for i in pivot_left..n.saturating_sub(pivot_right) {
        let h = high[i];
        if !crate::types::is_finite(h) {
            continue;
        }
        let is_swing_high = (i - pivot_left..i).all(|j| {
            crate::types::is_finite(high[j]) && high[j] < h
        }) && (i + 1..=i + pivot_right).all(|j| {
            j < n && crate::types::is_finite(high[j]) && high[j] < h
        });
        swing_high[i] = is_swing_high;
    }

    // 识别 swing_low
    for i in pivot_left..n.saturating_sub(pivot_right) {
        let l = low[i];
        if !crate::types::is_finite(l) {
            continue;
        }
        let is_swing_low = (i - pivot_left..i).all(|j| {
            crate::types::is_finite(low[j]) && low[j] > l
        }) && (i + 1..=i + pivot_right).all(|j| {
            j < n && crate::types::is_finite(low[j]) && low[j] > l
        });
        swing_low[i] = is_swing_low;
    }

    // 收集最近的 swing points
    let lookback_start = n.saturating_sub(structure_lookback);
    let recent_swing_highs: Vec<(usize, f64)> = (lookback_start..n.saturating_sub(pivot_right))
        .filter(|&i| swing_high[i])
        .map(|i| (i, high[i]))
        .collect();

    let recent_swing_lows: Vec<(usize, f64)> = (lookback_start..n.saturating_sub(pivot_right))
        .filter(|&i| swing_low[i])
        .map(|i| (i, low[i]))
        .collect();

    // HH/HL/LH/LL 判断
    let has_higher_high = if recent_swing_highs.len() >= 2 {
        let (_, last) = recent_swing_highs.last().unwrap();
        recent_swing_highs.iter().rev().skip(1).any(|(_, h)| *last > *h)
    } else {
        false
    };

    let has_lower_low = if recent_swing_lows.len() >= 2 {
        let (_, last) = recent_swing_lows.last().unwrap();
        recent_swing_lows.iter().rev().skip(1).any(|(_, l)| *last < *l)
    } else {
        false
    };

    let has_higher_low = if recent_swing_lows.len() >= 2 {
        let (_, last) = recent_swing_lows.last().unwrap();
        recent_swing_lows.iter().rev().skip(1).any(|(_, l)| *last > *l)
    } else {
        false
    };

    let has_lower_high = if recent_swing_highs.len() >= 2 {
        let (_, last) = recent_swing_highs.last().unwrap();
        recent_swing_highs.iter().rev().skip(1).any(|(_, h)| *last < *h)
    } else {
        false
    };

    // range_high / range_low：最近 lookback 内的已确认 swing 高低点
    let range_high = recent_swing_highs.iter().map(|(_, h)| *h).fold(f64::NEG_INFINITY, f64::max);
    let range_low = recent_swing_lows.iter().map(|(_, l)| *l).fold(f64::INFINITY, f64::min);

    PriceStructureResult {
        swing_high,
        swing_low,
        has_higher_high,
        has_higher_low,
        has_lower_high,
        has_lower_low,
        range_high: if range_high.is_finite() { Some(range_high) } else { None },
        range_low: if range_low.is_finite() { Some(range_low) } else { None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swing_high_detection() {
        let high = vec![10.0, 12.0, 11.0, 10.0, 13.0, 12.0, 11.0, 10.0, 12.0, 11.0];
        let low = vec![8.0; 10];
        let result = identify_price_structure(&high, &low, 2, 2, 20);
        // index 4 (value 13.0) should be a swing high
        assert!(result.swing_high[4], "expected swing_high at index 4");
    }

    #[test]
    fn test_swing_low_detection() {
        let high = vec![20.0; 10];
        let low = vec![10.0, 12.0, 9.0, 8.0, 11.0, 13.0, 12.0, 11.0, 10.0, 12.0];
        let result = identify_price_structure(&high, &low, 2, 2, 20);
        // index 3 (value 8.0) should be a swing low
        assert!(result.swing_low[3], "expected swing_low at index 3");
    }
}
