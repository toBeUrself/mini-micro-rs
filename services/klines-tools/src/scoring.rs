//! 三维评分系统：range_score / up_score / down_score。
//!
//! 每个评分 0~100，分子维度计算后乘权重求和。

use indicators::linear_down;
use crate::config::{IndicatorConfig, StateConfig};
use crate::models::{ScoreDetail, ScoreBreakdown, Scores, ScoreMomentum, IndicatorResults};

/// 计算原始三维评分。
///
/// - `ind`：指标计算结果
/// - `last_idx`：使用哪个索引的值（通常是最新已闭合 K 线的索引）
/// - `ic`：指标配置
/// - `sc`：状态配置
/// - `enable_score_momentum`：是否计算评分动能（暂用于返回值标记）
pub fn compute_raw_scores(
    ind: &IndicatorResults,
    last_idx: usize,
    ic: &IndicatorConfig,
    sc: &StateConfig,
    enable_score_conflict: bool,
) -> (Scores, ScoreBreakdown) {
    let mut range_details: Vec<ScoreDetail> = Vec::new();
    let mut up_details: Vec<ScoreDetail> = Vec::new();
    let mut down_details: Vec<ScoreDetail> = Vec::new();

    // ── range_score ────────────────────────────────────────────────
    let trend_weak = score_trend_weak(ind, last_idx);
    let vol_adapt = score_volatility_adapt(ind, last_idx);
    let ma_sticky = score_ma_sticky(ind, last_idx);
    let price_round = score_price_roundtrip(ind, last_idx, ic);
    let rsi_neutral = score_rsi_neutral(ind, last_idx);
    let vol_stable = score_volume_stable(ind, last_idx);
    let cost_adapt = score_cost_adapt(ind, last_idx);
    // weights: 25, 20, 15, 15, 10, 10, 5
    let range_raw = trend_weak.val * 0.25 + vol_adapt.val * 0.20 + ma_sticky.val * 0.15
        + price_round.val * 0.15 + rsi_neutral.val * 0.10 + vol_stable.val * 0.10 + cost_adapt.val * 0.05;

    range_details.push(trend_weak.sd(0.25));
    range_details.push(vol_adapt.sd(0.20));
    range_details.push(ma_sticky.sd(0.15));
    range_details.push(price_round.sd(0.15));
    range_details.push(rsi_neutral.sd(0.10));
    range_details.push(vol_stable.sd(0.10));
    range_details.push(cost_adapt.sd(0.05));

    // ── up_score ───────────────────────────────────────────────────
    let price_dir_up = score_price_direction_up(ind, last_idx);
    let momentum_up = score_momentum_up(ind, last_idx);
    let trend_up = score_trend_up(ind, last_idx);
    let breakout_up = score_breakout_up(ind, last_idx);
    let vol_confirm_up = score_volume_confirm_up(ind, last_idx);
    let structure_up = score_structure_up(ind, last_idx);
    // weights: 20, 20, 20, 15, 15, 10
    let up_raw = price_dir_up.val * 0.20 + momentum_up.val * 0.20 + trend_up.val * 0.20
        + breakout_up.val * 0.15 + vol_confirm_up.val * 0.15 + structure_up.val * 0.10;

    up_details.push(price_dir_up.sd(0.20));
    up_details.push(momentum_up.sd(0.20));
    up_details.push(trend_up.sd(0.20));
    up_details.push(breakout_up.sd(0.15));
    up_details.push(vol_confirm_up.sd(0.15));
    up_details.push(structure_up.sd(0.10));

    // ── down_score ─────────────────────────────────────────────────
    let price_dir_down = score_price_direction_down(ind, last_idx);
    let momentum_down = score_momentum_down(ind, last_idx);
    let trend_down = score_trend_down(ind, last_idx);
    let breakout_down = score_breakout_down(ind, last_idx);
    let vol_confirm_down = score_volume_confirm_down(ind, last_idx);
    let structure_down = score_structure_down(ind, last_idx);
    // weights: 20, 20, 20, 15, 15, 10
    let down_raw = price_dir_down.val * 0.20 + momentum_down.val * 0.20 + trend_down.val * 0.20
        + breakout_down.val * 0.15 + vol_confirm_down.val * 0.15 + structure_down.val * 0.10;

    down_details.push(price_dir_down.sd(0.20));
    down_details.push(momentum_down.sd(0.20));
    down_details.push(trend_down.sd(0.20));
    down_details.push(breakout_down.sd(0.15));
    down_details.push(vol_confirm_down.sd(0.15));
    down_details.push(structure_down.sd(0.10));

    let mut range_score = range_raw.clamp(0.0, 100.0);
    let up_score = up_raw.clamp(0.0, 100.0);
    let down_score = down_raw.clamp(0.0, 100.0);

    // 互斥修正
    if enable_score_conflict {
        if up_score >= sc.warning_enter || down_score >= sc.warning_enter {
            range_score *= 0.7;
        }
    }

    let scores = Scores { range_score, up_score, down_score };
    let breakdown = ScoreBreakdown {
        range: range_details,
        up: up_details,
        down: down_details,
    };

    (scores, breakdown)
}

/// 计算平滑评分（EMA 平滑）。
pub fn smooth_scores(raw: &[Scores], period: usize) -> Vec<Scores> {
    if raw.is_empty() || period == 0 {
        return raw.to_vec();
    }
    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut smoothed = Vec::with_capacity(raw.len());
    for (i, s) in raw.iter().enumerate() {
        if i == 0 {
            smoothed.push(s.clone());
        } else {
            let prev = &smoothed[i - 1];
            smoothed.push(Scores {
                range_score: (s.range_score - prev.range_score) * multiplier + prev.range_score,
                up_score: (s.up_score - prev.up_score) * multiplier + prev.up_score,
                down_score: (s.down_score - prev.down_score) * multiplier + prev.down_score,
            });
        }
    }
    smoothed
}

/// 计算评分动能。
pub fn score_momentum(current: &Scores, previous: &Scores) -> ScoreMomentum {
    ScoreMomentum {
        range_momentum: current.range_score - previous.range_score,
        up_momentum: current.up_score - previous.up_score,
        down_momentum: current.down_score - previous.down_score,
    }
}

// ── 辅助评分类型 ──────────────────────────────────────────────────────

struct SubScore {
    val: f64,
    name: String,
    raw_value: Option<f64>,
    available: bool,
    reason: String,
}

impl SubScore {
    fn available(name: &str, val: f64, raw: Option<f64>, reason: &str) -> Self {
        Self { val, name: name.into(), raw_value: raw, available: true, reason: reason.into() }
    }
    fn unavailable(name: &str, reason: &str) -> Self {
        Self { val: 0.0, name: name.into(), raw_value: None, available: false, reason: reason.into() }
    }
    fn sd(&self, weight: f64) -> ScoreDetail {
        ScoreDetail {
            name: self.name.clone(),
            raw_value: self.raw_value,
            sub_score: Some(self.val),
            weight,
            weighted_score: Some(self.val * weight),
            available: self.available,
            reason: self.reason.clone(),
        }
    }
}

fn get(v: &[f64], idx: usize) -> Option<f64> {
    v.get(idx).copied().filter(|x| x.is_finite())
}

// ── range_score 子维度 ────────────────────────────────────────────────

fn score_trend_weak(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.adx, idx) {
        Some(adx) => {
            let val = if adx <= 18.0 { 100.0 }
            else if adx < 25.0 { linear_down(adx, 18.0, 25.0) * 0.6 + 40.0 }
            else if adx < 35.0 { linear_down(adx, 25.0, 35.0) * 0.4 }
            else { 0.0 };
            SubScore::available("趋势弱", val.clamp(0.0, 100.0), Some(adx), &format!("ADX={adx:.1}"))
        }
        None => SubScore::unavailable("趋势弱", "ADX unavailable"),
    }
}

fn score_volatility_adapt(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.boll_bandwidth, idx) {
        Some(bbw) => {
            // Simplified: use fixed thresholds instead of percentile (percentile needs long history)
            let val = if bbw < 0.01 { 40.0 }
            else if bbw < 0.05 { 100.0 }
            else if bbw < 0.10 { 50.0 }
            else { 0.0 };
            SubScore::available("波动适配", val, Some(bbw), &format!("BOLL BW={bbw:.4}"))
        }
        None => SubScore::unavailable("波动适配", "BOLL bandwidth unavailable"),
    }
}

fn score_ma_sticky(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.ma_spread, idx) {
        Some(spread) => {
            let val = if spread <= 0.01 { 100.0 }
            else if spread < 0.03 { linear_down(spread, 0.01, 0.03) * 0.6 + 40.0 }
            else if spread > 0.05 { 0.0 }
            else { linear_down(spread, 0.03, 0.05) * 0.4 };
            SubScore::available("均线粘合", val.clamp(0.0, 100.0), Some(spread), &format!("MA spread={spread:.4}"))
        }
        None => SubScore::unavailable("均线粘合", "MA spread unavailable"),
    }
}

fn score_price_roundtrip(_ind: &IndicatorResults, _idx: usize, _ic: &IndicatorConfig) -> SubScore {
    // Simplified: check if recent prices cross MA20/BOLL mid multiple times
    SubScore::available("价格往返", 70.0, None, "simplified: default neutral")
}

fn score_rsi_neutral(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.rsi, idx) {
        Some(rsi) => {
            let val = if rsi >= 45.0 && rsi <= 55.0 { 100.0 }
            else if rsi >= 40.0 && rsi <= 60.0 { 80.0 }
            else if rsi >= 30.0 && rsi <= 70.0 { 40.0 }
            else { 0.0 };
            SubScore::available("RSI中性", val, Some(rsi), &format!("RSI={rsi:.1}"))
        }
        None => SubScore::unavailable("RSI中性", "RSI unavailable"),
    }
}

fn score_volume_stable(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.volume_ratio, idx) {
        Some(vr) => {
            let val = if vr >= 0.8 && vr <= 1.2 { 100.0 }
            else if vr >= 0.6 && vr <= 1.5 { 60.0 }
            else if vr > 1.5 { 20.0 }
            else { 0.0 };
            SubScore::available("成交平稳", val, Some(vr), &format!("VolRatio={vr:.2}"))
        }
        None => SubScore::unavailable("成交平稳", "Volume Ratio unavailable"),
    }
}

fn score_cost_adapt(_ind: &IndicatorResults, _idx: usize) -> SubScore {
    // Simplified: assume cost is adequate
    SubScore::available("成本适配", 100.0, None, "simplified: assume adequate")
}

// ── up_score 子维度 ───────────────────────────────────────────────────

fn score_price_direction_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    let ma20_slope = get(&ind.ma20_slope, idx);
    let percent_b = get(&ind.percent_b, idx);
    match (ma20_slope, percent_b) {
        (Some(slope), Some(pb)) => {
            let mut val = 0.0_f64;
            if slope > 0.001 { val += 60.0; }
            if slope > 0.0 { val += 20.0; }
            if pb > 0.8 { val += 20.0; }
            SubScore::available("价格方向(上)", val.clamp(0.0, 100.0), Some(slope), &format!("MA20 slope={slope:.4}, %B={pb:.2}"))
        }
        _ => SubScore::unavailable("价格方向(上)", "MA20 slope or %B unavailable"),
    }
}

fn score_momentum_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    match (get(&ind.macd_hist, idx), ind.macd_golden_cross.get(idx)) {
        (Some(hist), _) if hist > 0.0 => {
            // Check if hist is increasing
            let prev_hist = get(&ind.macd_hist, idx.saturating_sub(1)).unwrap_or(hist);
            let val = if hist > prev_hist { 90.0 } else { 50.0 };
            if *ind.macd_golden_cross.get(idx).unwrap_or(&false) {
                SubScore::available("动能增强", (val + 10.0_f64).min(100.0), Some(hist), "MACD hist>0, golden cross")
            } else {
                SubScore::available("动能增强", val, Some(hist), "MACD hist>0")
            }
        }
        (Some(hist), _) => {
            SubScore::available("动能增强", 10.0, Some(hist), "MACD hist<=0")
        }
        _ => SubScore::unavailable("动能增强", "MACD unavailable"),
    }
}

fn score_trend_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    match (get(&ind.plus_di, idx), get(&ind.minus_di, idx), get(&ind.adx, idx)) {
        (Some(pdi), Some(mdi), Some(adx)) => {
            let mut val = 0.0_f64;
            if pdi > mdi { val += 60.0; }
            // check if ADX is rising
            let prev_adx = get(&ind.adx, idx.saturating_sub(1)).unwrap_or(adx);
            if adx > prev_adx { val += 40.0; }
            SubScore::available("趋势增强(上)", val.clamp(0.0, 100.0), Some(adx), &format!("+DI={pdi:.1}, -DI={mdi:.1}, ADX={adx:.1}"))
        }
        _ => SubScore::unavailable("趋势增强(上)", "DI/ADX unavailable"),
    }
}

fn score_breakout_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    match (get(&ind.boll_upper, idx), get(&ind.donchian_upper, idx)) {
        (Some(bu), _) => {
            // Use %B as breakout proxy
            let pb = get(&ind.percent_b, idx).unwrap_or(0.5);
            let val = if pb > 1.0 { 80.0 } else if pb > 0.8 { 60.0 } else if pb > 0.5 { 30.0 } else { 10.0 };
            SubScore::available("突破确认(上)", val, Some(pb), &format!("%B={pb:.2}, BOLL upper={bu:.2}"))
        }
        _ => SubScore::unavailable("突破确认(上)", "BOLL unavailable"),
    }
}

fn score_volume_confirm_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.volume_ratio, idx) {
        Some(vr) => {
            let val = if vr >= 1.5 { 100.0 }
            else if vr >= 1.2 { 70.0 }
            else if vr >= 0.8 { 40.0 }
            else { 20.0 };
            SubScore::available("成交量确认(上)", val, Some(vr), &format!("VolRatio={vr:.2}"))
        }
        None => SubScore::unavailable("成交量确认(上)", "Volume Ratio unavailable"),
    }
}

fn score_structure_up(_ind: &IndicatorResults, _idx: usize) -> SubScore {
    // Price structure is complex; simplified default
    SubScore::available("价格结构(上)", 50.0, None, "simplified: neutral")
}

// ── down_score 子维度 ─────────────────────────────────────────────────

fn score_price_direction_down(ind: &IndicatorResults, idx: usize) -> SubScore {
    let ma20_slope = get(&ind.ma20_slope, idx);
    let percent_b = get(&ind.percent_b, idx);
    match (ma20_slope, percent_b) {
        (Some(slope), Some(pb)) => {
            let mut val = 0.0_f64;
            if slope < -0.001 { val += 60.0; }
            if slope < 0.0 { val += 20.0; }
            if pb < 0.2 { val += 20.0; }
            SubScore::available("价格方向(下)", val.clamp(0.0, 100.0), Some(slope), &format!("MA20 slope={slope:.4}, %B={pb:.2}"))
        }
        _ => SubScore::unavailable("价格方向(下)", "MA20 slope or %B unavailable"),
    }
}

fn score_momentum_down(ind: &IndicatorResults, idx: usize) -> SubScore {
    match (get(&ind.macd_hist, idx), ind.macd_death_cross.get(idx)) {
        (Some(hist), _) if hist < 0.0 => {
            let prev_hist = get(&ind.macd_hist, idx.saturating_sub(1)).unwrap_or(hist);
            let val = if hist < prev_hist { 90.0 } else { 50.0 };
            if *ind.macd_death_cross.get(idx).unwrap_or(&false) {
                SubScore::available("动能转弱", (val + 10.0_f64).min(100.0), Some(hist), "MACD hist<0, death cross")
            } else {
                SubScore::available("动能转弱", val, Some(hist), "MACD hist<0")
            }
        }
        (Some(hist), _) => {
            SubScore::available("动能转弱", 10.0, Some(hist), "MACD hist>=0")
        }
        _ => SubScore::unavailable("动能转弱", "MACD unavailable"),
    }
}

fn score_trend_down(ind: &IndicatorResults, idx: usize) -> SubScore {
    match (get(&ind.plus_di, idx), get(&ind.minus_di, idx), get(&ind.adx, idx)) {
        (Some(pdi), Some(mdi), Some(adx)) => {
            let mut val = 0.0_f64;
            if mdi > pdi { val += 60.0; }
            let prev_adx = get(&ind.adx, idx.saturating_sub(1)).unwrap_or(adx);
            if adx > prev_adx { val += 40.0; }
            SubScore::available("趋势增强(下)", val.clamp(0.0, 100.0), Some(adx), &format!("+DI={pdi:.1}, -DI={mdi:.1}, ADX={adx:.1}"))
        }
        _ => SubScore::unavailable("趋势增强(下)", "DI/ADX unavailable"),
    }
}

fn score_breakout_down(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.boll_lower, idx) {
        Some(bl) => {
            let pb = get(&ind.percent_b, idx).unwrap_or(0.5);
            let val = if pb < 0.0 { 80.0 } else if pb < 0.2 { 60.0 } else if pb < 0.5 { 30.0 } else { 10.0 };
            SubScore::available("破位确认(下)", val, Some(pb), &format!("%B={pb:.2}, BOLL lower={bl:.2}"))
        }
        None => SubScore::unavailable("破位确认(下)", "BOLL unavailable"),
    }
}

fn score_volume_confirm_down(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.volume_ratio, idx) {
        Some(vr) => {
            let val = if vr >= 1.5 { 100.0 }
            else if vr >= 1.2 { 70.0 }
            else if vr >= 0.8 { 40.0 }
            else { 20.0 };
            SubScore::available("成交量确认(下)", val, Some(vr), &format!("VolRatio={vr:.2}"))
        }
        None => SubScore::unavailable("成交量确认(下)", "Volume Ratio unavailable"),
    }
}

fn score_structure_down(_ind: &IndicatorResults, _idx: usize) -> SubScore {
    SubScore::available("价格结构(下)", 50.0, None, "simplified: neutral")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_score_high_in_consolidation() {
        // This would need real indicator data; placeholder
        assert!(linear_down(15.0, 18.0, 25.0) > 50.0);
    }
}
