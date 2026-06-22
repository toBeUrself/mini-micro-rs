//! K 线数据校验、排序、去重、缺失检查、闭合识别。

use crate::config::DataQualityConfig;
use crate::models::{DataQuality, GapRange, Kline};

/// 校验并清洗 K 线数据。
///
/// 步骤：
/// 1. OHLCV 合法性检查
/// 2. 排序
/// 3. 去重
/// 4. 缺失 K 线检测
/// 5. 闭合 K 线识别
/// 6. 延迟检测
/// 7. 生成 DataQuality 报告
///
/// 返回 (清洗后的 K线列表, DataQuality)。
/// 清洗后的 K线按 open_time 升序排列。
pub fn validate_and_clean(
    raw_klines: &[Kline],
    interval: &str,
    now_ms: i64,
    _dq_config: &DataQualityConfig,
) -> (Vec<Kline>, DataQuality) {
    let input_count = raw_klines.len();
    let interval_ms = crate::models::parse_interval_ms(interval);
    let mut issues: Vec<String> = Vec::new();

    if raw_klines.is_empty() {
        return (
            vec![],
            DataQuality {
                input_kline_count: 0,
                usable_closed_kline_count: 0,
                first_open_time: None,
                last_open_time: None,
                expected_interval_ms: interval_ms,
                missing_kline_count: 0,
                missing_kline_ratio: 0.0,
                max_gap_bars: 0,
                gap_ranges: vec![],
                duplicate_kline_count: 0,
                out_of_order_count: 0,
                invalid_ohlcv_count: 0,
                has_gap: false,
                has_unclosed_kline: false,
                latest_kline_delay_ms: 0,
                warmup_satisfied: false,
                quality_score: 0.0,
                issues: vec!["no data".into()],
            },
        );
    }

    // 1. OHLCV 合法性检查
    let mut invalid_count = 0;
    let mut valid: Vec<&Kline> = Vec::with_capacity(input_count);
    for k in raw_klines {
        if !k.open.is_finite() || !k.high.is_finite() || !k.low.is_finite() || !k.close.is_finite()
            || !k.volume.is_finite()
            || k.volume < 0.0
            || k.high < k.low
            || k.high < k.open
            || k.high < k.close
            || k.low > k.open
            || k.low > k.close
        {
            invalid_count += 1;
            continue;
        }
        // 独立检查 quote_volume，不影响 invalid_count（不重复计数）
        if let Some(qv) = k.quote_volume {
            if !qv.is_finite() || qv < 0.0 {
                issues.push(format!(
                    "kline at open_time {} has invalid quote_volume, marking as degraded",
                    k.open_time
                ));
            }
        }
        valid.push(k);
    }
    if invalid_count > 0 {
        issues.push(format!("{invalid_count} invalid OHLCV klines discarded"));
    }

    if valid.is_empty() {
        return (
            vec![],
            DataQuality {
                input_kline_count: input_count,
                usable_closed_kline_count: 0,
                first_open_time: None,
                last_open_time: None,
                expected_interval_ms: interval_ms,
                missing_kline_count: 0,
                missing_kline_ratio: 0.0,
                max_gap_bars: 0,
                gap_ranges: vec![],
                duplicate_kline_count: 0,
                out_of_order_count: 0,
                invalid_ohlcv_count: invalid_count,
                has_gap: false,
                has_unclosed_kline: false,
                latest_kline_delay_ms: 0,
                warmup_satisfied: false,
                quality_score: 0.0,
                issues,
            },
        );
    }

    // 2. 排序（按 open_time 升序）
    let mut sorted: Vec<&Kline> = valid;
    let out_of_order = is_out_of_order(&sorted);
    if out_of_order > 0 {
        issues.push(format!("{out_of_order} out-of-order klines sorted"));
    }
    sorted.sort_by_key(|k| k.open_time);

    // 3. 去重（保留最后出现的）
    let mut deduped: Vec<Kline> = Vec::with_capacity(sorted.len());
    let mut duplicates = 0;
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1].open_time == sorted[i].open_time {
            j += 1;
        }
        if j > i {
            duplicates += j - i;
            issues.push(format!(
                "{} duplicate klines at open_time {} kept last",
                j - i,
                sorted[i].open_time
            ));
        }
        deduped.push(sorted[j].clone());
        i = j + 1;
    }
    if duplicates > 0 {
        issues.push(format!("{duplicates} duplicate klines removed"));
    }

    // 4. 缺失 K 线检测
    let first_ot = deduped.first().map(|k| k.open_time);
    let last_ot = deduped.last().map(|k| k.open_time);
    let mut gap_ranges: Vec<GapRange> = Vec::new();
    let mut missing_total = 0usize;
    let mut max_gap = 0usize;

    if interval_ms > 0 {
        for w in deduped.windows(2) {
            let expected_next = w[0].open_time + interval_ms;
            let actual_next = w[1].open_time;
            if actual_next > expected_next {
                let missing = ((actual_next - expected_next) / interval_ms) as usize;
                missing_total += missing;
                max_gap = max_gap.max(missing);
                gap_ranges.push(GapRange {
                    expected_open_time: expected_next,
                    next_seen_open_time: actual_next,
                    missing_count: missing,
                });
            }
        }
    }

    let missing_ratio = if deduped.len() > 0 {
        missing_total as f64 / (deduped.len() + missing_total) as f64
    } else {
        0.0
    };

    // 5. 闭合 K 线识别
    let has_unclosed = deduped.iter().any(|k| !k.is_closed);

    // 6. 延迟检测：使用预期 close_time（open_time + interval_ms）
    let latest_delay = deduped
        .last()
        .map(|k| {
            let expected_close = k.open_time + interval_ms;
            (now_ms - expected_close).max(0)
        })
        .unwrap_or(0);

    // 7. Quality score
    let mut score: f64 = 1.0;
    if invalid_count > 0 {
        score *= 0.8_f64.powi(invalid_count.min(5) as i32);
    }
    if missing_ratio > 0.0 {
        score *= (1.0 - missing_ratio.min(0.5) * 2.0).max(0.1);
    }
    if has_unclosed {
        score *= 0.95;
    }
    let expected_latest = interval_ms;
    if latest_delay > expected_latest * 2 {
        score *= 0.7;
    } else if latest_delay > expected_latest {
        score *= 0.9;
    }
    let quality_score = score.clamp(0.0, 1.0);

    // warmup_satisfied: at least 60 bars for minimal analysis
    let warmup_satisfied = deduped.len() >= 60;

    let closed_count = deduped.iter().filter(|k| k.is_closed).count();

    let dq = DataQuality {
        input_kline_count: input_count,
        usable_closed_kline_count: closed_count,
        first_open_time: first_ot,
        last_open_time: last_ot,
        expected_interval_ms: interval_ms,
        missing_kline_count: missing_total,
        missing_kline_ratio: missing_ratio,
        max_gap_bars: max_gap,
        gap_ranges,
        duplicate_kline_count: duplicates,
        out_of_order_count: out_of_order,
        invalid_ohlcv_count: invalid_count,
        has_gap: missing_total > 0,
        has_unclosed_kline: has_unclosed,
        latest_kline_delay_ms: latest_delay,
        warmup_satisfied,
        quality_score,
        issues,
    };

    (deduped, dq)
}

/// 检查是否乱序。
fn is_out_of_order(klines: &[&Kline]) -> usize {
    let mut count = 0;
    for w in klines.windows(2) {
        if w[0].open_time > w[1].open_time {
            count += 1;
        }
    }
    count
}

/// 判断 K 线 open_time 是否与 interval 对齐。
/// 对于 1m interval，open_time 应当能被 60000ms 整除。
pub fn check_time_alignment(open_time: i64, interval_ms: i64) -> bool {
    if interval_ms <= 0 {
        return true;
    }
    open_time % interval_ms == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_kline(open_time: i64, high: f64, low: f64, is_closed: bool) -> Kline {
        Kline {
            open_time,
            interval: "5m".into(),
            open: (high + low) / 2.0,
            high,
            low,
            close: (high + low) / 2.0,
            volume: 100.0,
            quote_volume: None,
            close_time: None,
            trade_count: None,
            is_closed,
        }
    }

    #[test]
    fn test_reject_invalid_ohlcv() {
        let raw = vec![
            make_kline(1000, 100.0, 101.0, true), // high < low → invalid
            make_kline(2000, 101.0, 100.0, true), // valid
        ];
        let dq_config = DataQualityConfig::default();
        let (cleaned, dq) = validate_and_clean(&raw, "5m", 3000, &dq_config);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(dq.invalid_ohlcv_count, 1);
    }

    #[test]
    fn test_detect_duplicates() {
        let raw = vec![
            make_kline(1000, 101.0, 100.0, true),
            make_kline(1000, 101.5, 100.0, true), // duplicate
            make_kline(2000, 102.0, 101.0, true),
        ];
        let dq_config = DataQualityConfig::default();
        let (cleaned, dq) = validate_and_clean(&raw, "5m", 3000, &dq_config);
        assert_eq!(cleaned.len(), 2);
        assert!(dq.duplicate_kline_count > 0);
    }

    #[test]
    fn test_gap_detection() {
        let raw = vec![
            make_kline(0, 101.0, 100.0, true),
            make_kline(300000, 102.0, 101.0, true), // 5min interval = 300000ms
            make_kline(900000, 103.0, 102.0, true), // gap: should be 600000
        ];
        let dq_config = DataQualityConfig::default();
        let (_cleaned, dq) = validate_and_clean(&raw, "5m", 1000000, &dq_config);
        assert!(dq.has_gap);
        assert!(dq.missing_kline_count > 0);
    }

    #[test]
    fn test_unclosed_detection() {
        let raw = vec![
            make_kline(1000, 101.0, 100.0, true),
            make_kline(6000, 102.0, 101.0, false), // unclosed
        ];
        let dq_config = DataQualityConfig::default();
        let (_, dq) = validate_and_clean(&raw, "5m", 12000, &dq_config);
        assert!(dq.has_unclosed_kline);
    }
}
