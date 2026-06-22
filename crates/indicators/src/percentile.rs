//! 滚动分位数计算。
//!
//! 用于对指标值进行历史归一化，例如：
//! - < 15%：极低
//! - 15%~30%：偏低
//! - 30%~70%：正常
//! - 70%~85%：偏高
//! - > 85%：极高

/// 计算一个值在窗口中的分位数（0.0 ~ 1.0）。
///
/// 使用线性插值计算分位数排名。
/// 返回 (percentile, sample_count)。
pub fn percentile_of(values: &[f64], target: f64) -> (f64, usize) {
    if values.is_empty() {
        return (0.0, 0);
    }

    let mut sorted: Vec<f64> = values
        .iter()
        .filter(|v| v.is_finite())
        .copied()
        .collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = sorted.len();
    if n == 0 {
        return (0.0, 0);
    }

    // 线性插值分位数
    match sorted.binary_search_by(|v| v.partial_cmp(&target).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(idx) => {
            // 精确匹配，取匹配区间的中点位置
            let mut first = idx;
            while first > 0 && (sorted[first - 1] - target).abs() < f64::EPSILON {
                first -= 1;
            }
            let mut last = idx;
            while last + 1 < n && (sorted[last + 1] - target).abs() < f64::EPSILON {
                last += 1;
            }
            let mid = (first + last) as f64 / 2.0;
            (mid / (n - 1).max(1) as f64, n)
        }
        Err(insert_idx) => {
            if insert_idx == 0 {
                (0.0, n)
            } else if insert_idx >= n {
                (1.0, n)
            } else {
                // 线性插值
                let lower = sorted[insert_idx - 1];
                let upper = sorted[insert_idx];
                let fraction = if (upper - lower).abs() > f64::EPSILON {
                    (target - lower) / (upper - lower)
                } else {
                    0.5
                };
                let pos = (insert_idx - 1) as f64 + fraction;
                (pos / (n - 1).max(1) as f64, n)
            }
        }
    }
}

/// 计算滑动窗口中每个位置的分位数。
///
/// 返回每个位置 target[i] 在窗口 values[i-window+1..=i] 中的分位数。
pub fn rolling_percentile(values: &[f64], target: &[f64], window: usize) -> Vec<Option<f64>> {
    let n = values.len().min(target.len());
    let mut result = Vec::with_capacity(n);

    for i in 0..n {
        let start = i.saturating_sub(window.saturating_sub(1));
        let window_data = &values[start..=i];
        let (pct, _) = percentile_of(window_data, target[i]);
        if window_data.is_empty() || !window_data.iter().any(|v| v.is_finite()) {
            result.push(None);
        } else {
            result.push(Some(pct));
        }
    }

    result
}

/// 分位数分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentileCategory {
    VeryLow,   // < 15%
    Low,       // 15% ~ 30%
    Normal,    // 30% ~ 70%
    High,      // 70% ~ 85%
    VeryHigh,  // > 85%
}

impl PercentileCategory {
    pub fn classify(percentile: f64) -> Self {
        match percentile {
            p if p < 0.15 => Self::VeryLow,
            p if p < 0.30 => Self::Low,
            p if p < 0.70 => Self::Normal,
            p if p < 0.85 => Self::High,
            _ => Self::VeryHigh,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_of() {
        let values: Vec<f64> = (0..100).map(|x| x as f64).collect();
        let (pct, n) = percentile_of(&values, 0.0);
        assert!((pct - 0.0).abs() < 0.01);
        assert_eq!(n, 100);

        let (pct, _) = percentile_of(&values, 99.0);
        assert!((pct - 1.0).abs() < 0.01);

        let (pct, _) = percentile_of(&values, 49.5);
        assert!((pct - 0.5).abs() < 0.05);
    }

    #[test]
    fn test_percentile_category() {
        assert_eq!(PercentileCategory::classify(0.10), PercentileCategory::VeryLow);
        assert_eq!(PercentileCategory::classify(0.20), PercentileCategory::Low);
        assert_eq!(PercentileCategory::classify(0.50), PercentileCategory::Normal);
        assert_eq!(PercentileCategory::classify(0.80), PercentileCategory::High);
        assert_eq!(PercentileCategory::classify(0.90), PercentileCategory::VeryHigh);
    }
}
