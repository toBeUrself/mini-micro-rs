//! 三维评分系统：range_score / up_score / down_score。
//!
//! 每个评分 0~100，分子维度计算后乘权重求和。

use indicators::{linear_down, linear_up};
use crate::config::{IndicatorConfig, StateConfig};
use crate::models::{IndicatorResults, ScoreBreakdown, ScoreDetail, ScoreMomentum, Scores};

/// 计算原始三维评分。
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

    let trend_weak = score_trend_weak(ind, last_idx);
    let vol_adapt = score_volatility_adapt(ind, last_idx);
    let ma_sticky = score_ma_sticky(ind, last_idx);
    let price_round = score_price_roundtrip(ind, last_idx, ic);
    let rsi_neutral = score_rsi_neutral(ind, last_idx);
    let vol_stable = score_volume_stable(ind, last_idx);
    let cost_adapt = score_cost_adapt(ind, last_idx);

    let range_raw = trend_weak.val * 0.25
        + vol_adapt.val * 0.20
        + ma_sticky.val * 0.15
        + price_round.val * 0.15
        + rsi_neutral.val * 0.10
        + vol_stable.val * 0.10
        + cost_adapt.val * 0.05;

    range_details.push(trend_weak.sd(0.25));
    range_details.push(vol_adapt.sd(0.20));
    range_details.push(ma_sticky.sd(0.15));
    range_details.push(price_round.sd(0.15));
    range_details.push(rsi_neutral.sd(0.10));
    range_details.push(vol_stable.sd(0.10));
    range_details.push(cost_adapt.sd(0.05));

    let price_dir_up = score_price_direction_up(ind, last_idx);
    let momentum_up = score_momentum_up(ind, last_idx);
    let trend_up = score_trend_up(ind, last_idx);
    let breakout_up = score_breakout_up(ind, last_idx);
    let vol_confirm_up = score_volume_confirm_up(ind, last_idx);
    let structure_up = score_structure_up(ind, last_idx, ic.structure_lookback);

    let up_raw = price_dir_up.val * 0.20
        + momentum_up.val * 0.20
        + trend_up.val * 0.20
        + breakout_up.val * 0.15
        + vol_confirm_up.val * 0.15
        + structure_up.val * 0.10;

    up_details.push(price_dir_up.sd(0.20));
    up_details.push(momentum_up.sd(0.20));
    up_details.push(trend_up.sd(0.20));
    up_details.push(breakout_up.sd(0.15));
    up_details.push(vol_confirm_up.sd(0.15));
    up_details.push(structure_up.sd(0.10));

    let price_dir_down = score_price_direction_down(ind, last_idx);
    let momentum_down = score_momentum_down(ind, last_idx);
    let trend_down = score_trend_down(ind, last_idx);
    let breakout_down = score_breakout_down(ind, last_idx);
    let vol_confirm_down = score_volume_confirm_down(ind, last_idx);
    let structure_down = score_structure_down(ind, last_idx, ic.structure_lookback);

    let down_raw = price_dir_down.val * 0.20
        + momentum_down.val * 0.20
        + trend_down.val * 0.20
        + breakout_down.val * 0.15
        + vol_confirm_down.val * 0.15
        + structure_down.val * 0.10;

    down_details.push(price_dir_down.sd(0.20));
    down_details.push(momentum_down.sd(0.20));
    down_details.push(trend_down.sd(0.20));
    down_details.push(breakout_down.sd(0.15));
    down_details.push(vol_confirm_down.sd(0.15));
    down_details.push(structure_down.sd(0.10));

    let mut range_score = range_raw.clamp(0.0, 100.0);
    let up_score = up_raw.clamp(0.0, 100.0);
    let down_score = down_raw.clamp(0.0, 100.0);

    if enable_score_conflict && (up_score >= sc.warning_enter || down_score >= sc.warning_enter) {
        range_score *= 0.7;
        range_details.push(ScoreDetail {
            name: "互斥修正".into(),
            raw_value: Some(up_score.max(down_score)),
            sub_score: Some(range_score),
            weight: 0.0,
            weighted_score: Some(0.0),
            available: true,
            reason: "up_score 或 down_score 达到 warning_enter，range_score * 0.7".into(),
        });
    }

    let scores = Scores { range_score, up_score, down_score };
    let breakdown = ScoreBreakdown { range: range_details, up: up_details, down: down_details };

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
            sub_score: if self.available { Some(self.val) } else { None },
            weight,
            weighted_score: if self.available { Some(self.val * weight) } else { None },
            available: self.available,
            reason: self.reason.clone(),
        }
    }
}

fn get(v: &[f64], idx: usize) -> Option<f64> {
    v.get(idx).copied().filter(|x| x.is_finite())
}

fn score_trend_weak(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.adx, idx) {
        Some(adx) => {
            let val = if adx <= 18.0 {
                100.0
            } else if adx < 25.0 {
                40.0 + linear_down(adx, 18.0, 25.0) * 0.6
            } else if adx < 35.0 {
                linear_down(adx, 25.0, 35.0) * 0.4
            } else {
                0.0
            };
            SubScore::available("趋势弱", val.clamp(0.0, 100.0), Some(adx), &format!("ADX={adx:.1}"))
        }
        None => SubScore::unavailable("趋势弱", "ADX unavailable"),
    }
}

fn score_volatility_adapt(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.boll_bandwidth, idx) {
        Some(bbw) => {
            let val = if bbw < 0.01 {
                40.0
            } else if bbw < 0.05 {
                100.0
            } else if bbw < 0.10 {
                50.0
            } else {
                0.0
            };
            SubScore::available("波动适配", val, Some(bbw), &format!("BOLL BW={bbw:.4}"))
        }
        None => SubScore::unavailable("波动适配", "BOLL bandwidth unavailable"),
    }
}

fn score_ma_sticky(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.ma_spread, idx) {
        Some(spread) => {
            let val = if spread <= 0.01 {
                100.0
            } else if spread < 0.03 {
                40.0 + linear_down(spread, 0.01, 0.03) * 0.6
            } else if spread > 0.05 {
                0.0
            } else {
                linear_down(spread, 0.03, 0.05) * 0.4
            };
            SubScore::available("均线粘合", val.clamp(0.0, 100.0), Some(spread), &format!("MA spread={spread:.4}"))
        }
        None => SubScore::unavailable("均线粘合", "MA spread unavailable"),
    }
}

fn score_price_roundtrip(ind: &IndicatorResults, idx: usize, _ic: &IndicatorConfig) -> SubScore {
    if idx < 5 {
        return SubScore::unavailable("价格往返", "insufficient bars");
    }
    let lookback = 20.min(idx + 1);
    let start = idx + 1 - lookback;
    let mut crosses = 0usize;
    let mut prev_side: Option<i8> = None;

    for i in start..=idx {
        let Some(close) = get(&ind.close, i) else { continue; };
        let mid = get(&ind.boll_mid, i).or_else(|| get(&ind.ma20, i));
        let Some(mid) = mid else { continue; };
        let side = if close >= mid { 1 } else { -1 };
        if let Some(prev) = prev_side {
            if prev != side {
                crosses += 1;
            }
        }
        prev_side = Some(side);
    }

    let val = if crosses >= 3 { 100.0 } else { crosses as f64 / 3.0 * 100.0 };
    SubScore::available("价格往返", val, Some(crosses as f64), &format!("midline crosses={crosses}"))
}

fn score_rsi_neutral(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.rsi, idx) {
        Some(rsi) => {
            let val = if (45.0..=55.0).contains(&rsi) {
                100.0
            } else if (40.0..=60.0).contains(&rsi) {
                80.0
            } else if (30.0..=70.0).contains(&rsi) {
                40.0
            } else {
                0.0
            };
            SubScore::available("RSI中性", val, Some(rsi), &format!("RSI={rsi:.1}"))
        }
        None => SubScore::unavailable("RSI中性", "RSI unavailable"),
    }
}

fn score_volume_stable(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.volume_ratio, idx) {
        Some(vr) => {
            let val = if (0.8..=1.2).contains(&vr) {
                100.0
            } else if (0.6..=1.5).contains(&vr) {
                60.0
            } else if vr > 1.5 {
                20.0
            } else {
                0.0
            };
            SubScore::available("成交平稳", val, Some(vr), &format!("VolRatio={vr:.2}"))
        }
        None => SubScore::unavailable("成交平稳", "Volume Ratio unavailable"),
    }
}

fn score_cost_adapt(ind: &IndicatorResults, idx: usize) -> SubScore {
    let Some(close) = get(&ind.close, idx) else {
        return SubScore::unavailable("成本适配", "close unavailable");
    };
    let (Some(upper), Some(lower)) = (get(&ind.boll_upper, idx), get(&ind.boll_lower, idx)) else {
        return SubScore::unavailable("成本适配", "BOLL unavailable");
    };
    if close <= 0.0 || upper <= lower {
        return SubScore::unavailable("成本适配", "invalid price or BOLL width");
    }
    // 近似按 20 格网格估算单格空间，和默认手续费/滑点/缓冲的组合阈值比较。
    let step_pct = ((upper - lower) / 20.0) / close;
    let required = 0.0025;
    let val = if step_pct >= required { 100.0 } else { linear_up(step_pct, 0.0, required) };
    SubScore::available("成本适配", val, Some(step_pct), &format!("estimated grid step pct={step_pct:.5}"))
}

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
            let prev_hist = get(&ind.macd_hist, idx.saturating_sub(1)).unwrap_or(hist);
            let val = if hist > prev_hist { 90.0 } else { 50.0 };
            if *ind.macd_golden_cross.get(idx).unwrap_or(&false) {
                SubScore::available("动能增强", (val + 10.0_f64).min(100.0), Some(hist), "MACD hist>0, golden cross")
            } else {
                SubScore::available("动能增强", val, Some(hist), "MACD hist>0")
            }
        }
        (Some(hist), _) => SubScore::available("动能增强", 10.0, Some(hist), "MACD hist<=0"),
        _ => SubScore::unavailable("动能增强", "MACD unavailable"),
    }
}

fn score_trend_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    match (get(&ind.plus_di, idx), get(&ind.minus_di, idx), get(&ind.adx, idx)) {
        (Some(pdi), Some(mdi), Some(adx)) => {
            let mut val = 0.0_f64;
            if pdi > mdi { val += 60.0; }
            let prev_adx = get(&ind.adx, idx.saturating_sub(1)).unwrap_or(adx);
            if adx > prev_adx { val += 40.0; }
            SubScore::available("趋势增强(上)", val.clamp(0.0, 100.0), Some(adx), &format!("+DI={pdi:.1}, -DI={mdi:.1}, ADX={adx:.1}"))
        }
        _ => SubScore::unavailable("趋势增强(上)", "DI/ADX unavailable"),
    }
}

fn score_breakout_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    let close = get(&ind.close, idx);
    let pb = get(&ind.percent_b, idx);
    let boll_break = pb.map(|v| v > 1.0).unwrap_or(false);
    let donchian_break = if idx > 0 {
        match (close, get(&ind.donchian_upper, idx - 1)) {
            (Some(c), Some(prev_upper)) => c > prev_upper,
            _ => false,
        }
    } else {
        false
    };

    match (close, pb) {
        (Some(c), Some(pb)) => {
            let mut val = if pb > 1.0 { 70.0 } else if pb > 0.8 { 50.0 } else if pb > 0.5 { 25.0 } else { 5.0 };
            if boll_break { val += 10.0; }
            if donchian_break { val += 25.0; }
            SubScore::available(
                "突破确认(上)",
                val.clamp(0.0, 100.0),
                Some(c),
                &format!("%B={pb:.2}, donchian_break={donchian_break}"),
            )
        }
        _ => SubScore::unavailable("突破确认(上)", "close or %B unavailable"),
    }
}

fn score_volume_confirm_up(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.volume_ratio, idx) {
        Some(vr) => {
            let val = if vr >= 1.5 { 100.0 } else if vr >= 1.2 { 70.0 } else if vr >= 0.8 { 40.0 } else { 20.0 };
            SubScore::available("成交量确认(上)", val, Some(vr), &format!("VolRatio={vr:.2}"))
        }
        None => SubScore::unavailable("成交量确认(上)", "Volume Ratio unavailable"),
    }
}

fn score_structure_up(ind: &IndicatorResults, idx: usize, lookback: usize) -> SubScore {
    let lb = lookback.max(5).min(idx + 1);
    if lb < 5 {
        return SubScore::unavailable("价格结构(上)", "insufficient bars");
    }
    let start = idx + 1 - lb;
    let old_high = ind.high[start..start + lb / 2].iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    let new_high = ind.high[start + lb / 2..=idx].iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    let old_low = ind.low[start..start + lb / 2].iter().copied().filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    let new_low = ind.low[start + lb / 2..=idx].iter().copied().filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    if !old_high.is_finite() || !new_high.is_finite() || !old_low.is_finite() || !new_low.is_finite() {
        return SubScore::unavailable("价格结构(上)", "high/low unavailable");
    }
    let higher_high = new_high > old_high;
    let higher_low = new_low > old_low;
    let val = match (higher_high, higher_low) {
        (true, true) => 100.0,
        (true, false) | (false, true) => 50.0,
        _ => 10.0,
    };
    SubScore::available("价格结构(上)", val, Some(new_high - old_high), &format!("HH={higher_high}, HL={higher_low}"))
}

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
        (Some(hist), _) => SubScore::available("动能转弱", 10.0, Some(hist), "MACD hist>=0"),
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
    let close = get(&ind.close, idx);
    let pb = get(&ind.percent_b, idx);
    let boll_break = pb.map(|v| v < 0.0).unwrap_or(false);
    let donchian_break = if idx > 0 {
        match (close, get(&ind.donchian_lower, idx - 1)) {
            (Some(c), Some(prev_lower)) => c < prev_lower,
            _ => false,
        }
    } else {
        false
    };

    match (close, pb) {
        (Some(c), Some(pb)) => {
            let mut val = if pb < 0.0 { 70.0 } else if pb < 0.2 { 50.0 } else if pb < 0.5 { 25.0 } else { 5.0 };
            if boll_break { val += 10.0; }
            if donchian_break { val += 25.0; }
            SubScore::available(
                "破位确认(下)",
                val.clamp(0.0, 100.0),
                Some(c),
                &format!("%B={pb:.2}, donchian_break={donchian_break}"),
            )
        }
        _ => SubScore::unavailable("破位确认(下)", "close or %B unavailable"),
    }
}

fn score_volume_confirm_down(ind: &IndicatorResults, idx: usize) -> SubScore {
    match get(&ind.volume_ratio, idx) {
        Some(vr) => {
            let val = if vr >= 1.5 { 100.0 } else if vr >= 1.2 { 70.0 } else if vr >= 0.8 { 40.0 } else { 20.0 };
            SubScore::available("成交量确认(下)", val, Some(vr), &format!("VolRatio={vr:.2}"))
        }
        None => SubScore::unavailable("成交量确认(下)", "Volume Ratio unavailable"),
    }
}

fn score_structure_down(ind: &IndicatorResults, idx: usize, lookback: usize) -> SubScore {
    let lb = lookback.max(5).min(idx + 1);
    if lb < 5 {
        return SubScore::unavailable("价格结构(下)", "insufficient bars");
    }
    let start = idx + 1 - lb;
    let old_high = ind.high[start..start + lb / 2].iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    let new_high = ind.high[start + lb / 2..=idx].iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    let old_low = ind.low[start..start + lb / 2].iter().copied().filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    let new_low = ind.low[start + lb / 2..=idx].iter().copied().filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    if !old_high.is_finite() || !new_high.is_finite() || !old_low.is_finite() || !new_low.is_finite() {
        return SubScore::unavailable("价格结构(下)", "high/low unavailable");
    }
    let lower_high = new_high < old_high;
    let lower_low = new_low < old_low;
    let val = match (lower_high, lower_low) {
        (true, true) => 100.0,
        (true, false) | (false, true) => 50.0,
        _ => 10.0,
    };
    SubScore::available("价格结构(下)", val, Some(old_low - new_low), &format!("LH={lower_high}, LL={lower_low}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_score_helper() {
        assert!(linear_down(15.0, 18.0, 25.0) > 50.0);
    }

    #[test]
    fn test_smooth_scores() {
        let raw = vec![
            Scores { range_score: 10.0, up_score: 0.0, down_score: 0.0 },
            Scores { range_score: 20.0, up_score: 0.0, down_score: 0.0 },
        ];
        let smoothed = smooth_scores(&raw, 3);
        assert!(smoothed[1].range_score > 10.0 && smoothed[1].range_score < 20.0);
    }
}
