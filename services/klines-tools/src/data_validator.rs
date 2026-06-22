//! K 线数据校验、排序、去重、缺失检查、闭合识别。

use crate::config::DataQualityConfig;
use crate::models::{DataQuality, GapRange, Kline};

const MIN_WARMUP_BARS: usize = 60;

/// 校验并清洗 K 线数据。
///
/// 步骤：
/// 1. OHLCV 合法性检查
/// 2. 时间对齐检查
/// 3. 排序
/// 4. 去重
/// 5. 缺失 K 线检测
/// 6. 闭合 K 线识别
/// 7. 延迟检测
/// 8. 生成 DataQuality 报告
///
/// 返回 (清洗后的 K线列表, DataQuality)。清洗后的 K线按 open_time 升序排列。
pub fn validate_and_clean(
    raw_klines: &[Kline],
    interval: &str,
    now_ms: i64,
    dq_config: &DataQualityConfig,
) -> (Vec<Kline>, DataQuality) {
    let input_count = raw_klines.len();
    let interval_ms = crate::models::parse_interval_ms(interval);
    let mut issues: Vec<String> = Vec::new();

    if raw_klines.is_empty() {
        return (vec![], empty_quality(interval_ms, "no data"));
    }

    let mut invalid_count = 0usize;
    let mut misaligned_count = 0usize;
    let mut valid: Vec<&Kline> = Vec::with_capacity(input_count);

    for k in raw_klines {
        let quote_volume_invalid = k
            .quote_volume
            .map(|qv| !qv.is_finite() || qv < 0.0)
            .unwrap_or(false);

        let invalid = !k.open.is_finite()
            || !k.high.is_finite()
            || !k.low.is_finite()
            || !k.close.is_finite()
            || !k.volume.is_finite()
            || k.volume < 0.0
            || quote_volume_invalid
            || k.high < k.low
            || k.high < k.open
            || k.high < k.close
            || k.low > k.open
            || k.low > k.close;

        if invalid {
            invalid_count += 1;
            continue;
        }

        if !check_time_alignment(k.open_time, interval_ms) {
            misaligned_count += 1;
        }

        valid.push(k);
    }

    if invalid_count > 0 {
        issues.push(format!("{invalid_count} invalid OHLCV klines discarded"));
    }
    if misaligned_count > 0 {
        issues.push(format!("{misaligned_count} klines have misaligned open_time"));
    }

    if valid.is_empty() {
        let mut dq = empty_quality(interval_ms, "all klines invalid after OHLCV validation");
        dq.input_kline_count = input_count;
        dq.invalid_ohlcv_count = invalid_count;
        dq.issues = issues;
        dq.quality_score = 0.0;
        return (vec![], dq);
    }

    // 排序（按 open_time 升序）
    let mut sorted: Vec<&Kline> = valid;
    let out_of_order = is_out_of_order(&sorted);
    if out_of_order > 0 {
        issues.push(format!("{out_of_order} out-of-order adjacent pairs sorted"));
    }
    sorted.sort_by_key(|k| k.open_time);

    // 去重（保留最后出现的）
    let mut deduped: Vec<Kline> = Vec::with_capacity(sorted.len());
    let mut duplicates = 0usize;
    let mut i = 0usize;
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

    let missing_ratio = if !deduped.is_empty() {
        missing_total as f64 / (deduped.len() + missing_total) as f64
    } else {
        0.0
    };

    let closed_count = deduped.iter().filter(|k| k.is_closed).count();
    let has_unclosed = deduped.iter().any(|k| !k.is_closed);

    // latest delay 应以最新已闭合/最新 K 线的 close_time 为基准，而不是 open_time。
    let latest_delay = deduped
        .last()
        .map(|k| {
            let close_time = if interval_ms > 0 { k.open_time + interval_ms } else { k.open_time };
            (now_ms - close_time).max(0)
        })
        .unwrap_or(0);

    let warmup_satisfied = closed_count >= MIN_WARMUP_BARS;

    let mut score: f64 = 1.0;
    if invalid_count > 0 {
        score *= 0.8_f64.powi(invalid_count.min(5) as i32);
    }
    if misaligned_count > 0 {
        score *= 0.95_f64.powi(misaligned_count.min(5) as i32);
    }
    if missing_ratio > 0.0 {
        score *= (1.0 - missing_ratio.min(0.5) * 2.0).max(0.1);
    }
    if missing_ratio > dq_config.max_missing_kline_ratio {
        score *= 0.7;
        issues.push(format!(
            "missing ratio {:.4} exceeds configured max {:.4}",
            missing_ratio, dq_config.max_missing_kline_ratio
        ));
    }
    if has_unclosed {
        score *= 0.95;
    }
    if !warmup_satisfied {
        score *= 0.5;
        issues.push(format!("closed warmup bars insufficient: {closed_count}/{MIN_WARMUP_BARS}"));
    }

    if interval_ms > 0 {
        let max_delay = interval_ms * dq_config.max_latest_delay_intervals.max(1);
        if latest_delay > max_delay {
            score *= 0.7;
            issues.push(format!("latest kline delay {latest_delay}ms exceeds {max_delay}ms"));
        } else if latest_delay > interval_ms {
            score *= 0.9;
        }
    }

    let quality_score = score.clamp(0.0, 1.0);
    if quality_score < dq_config.min_quality_score_for_grid {
        issues.push(format!(
            "quality_score {:.3} below min_quality_score_for_grid {:.3}",
            quality_score, dq_config.min_quality_score_for_grid
        ));
    }

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

fn empty_quality(interval_ms: i64, issue: &str) -> DataQuality {
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
        issues: vec![issue.into()],
    }
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
            is_closed,
        }
    }

    #[test]
    fn test_reject_invalid_ohlcv() {
        let raw = vec![
            make_kline(0, 100.0, 101.0, true),
            make_kline(300000, 101.0, 100.0, true),
        ];
        let dq_config = DataQualityConfig::default();
        let (cleaned, dq) = validate_and_clean(&raw, "5m", 900000, &dq_config);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(dq.invalid_ohlcv_count, 1);
    }

    #[test]
    fn test_detect_duplicates() {
        let raw = vec![
            make_kline(0, 101.0, 100.0, true),
            make_kline(0, 101.5, 100.0, true),
            make_kline(300000, 102.0, 101.0, true),
        ];
        let dq_config = DataQualityConfig::default();
        let (cleaned, dq) = validate_and_clean(&raw, "5m", 900000, &dq_config);
        assert_eq!(cleaned.len(), 2);
        assert!(dq.duplicate_kline_count > 0);
    }

    #[test]
    fn test_gap_detection() {
        let raw = vec![
            make_kline(0, 101.0, 100.0, true),
            make_kline(300000, 102.0, 101.0, true),
            make_kline(900000, 103.0, 102.0, true),
        ];
        let dq_config = DataQualityConfig::default();
        let (_cleaned, dq) = validate_and_clean(&raw, "5m", 1_500_000, &dq_config);
        assert!(dq.has_gap);
        assert!(dq.missing_kline_count > 0);
    }

    #[test]
    fn test_unclosed_detection() {
        let raw = vec![
            make_kline(0, 101.0, 100.0, true),
            make_kline(300000, 102.0, 101.0, false),
        ];
        let dq_config = DataQualityConfig::default();
        let (_, dq) = validate_and_clean(&raw, "5m", 900000, &dq_config);
        assert!(dq.has_unclosed_kline);
    }
}
