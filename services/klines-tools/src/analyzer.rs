//! 主分析编排器：串联数据校验 → 指标计算 → 评分 → 状态机 → 风险 → 输出。

use std::{collections::HashMap, sync::{Arc, Mutex}};

use indicators::{adx, atr, boll, donchian, ma, macd, rsi, vol_ratio};
use indicators::types::IndicatorValue;

use crate::{
    config::KlinesToolsConfig,
    confidence, data_validator, grid_plan, kline_reader::KlineReader, multi_tf, output, risk,
    scoring, signal, state_machine,
    models::*,
};

/// 分析器：持有配置、K线读取器和内存状态上下文。
///
/// 状态上下文按 source+symbol+interval+config_hash 隔离，用于让 confirm_bars、cooldown、
/// smoothed_scores 和 score_momentum 在请求之间真实生效。
#[derive(Clone)]
pub struct Analyzer {
    pub config: KlinesToolsConfig,
    pub reader: KlineReader,
    state_store: Arc<Mutex<HashMap<String, StateContext>>>,
    score_store: Arc<Mutex<HashMap<String, Vec<Scores>>>>,
}

impl Analyzer {
    pub fn new(config: KlinesToolsConfig, reader: KlineReader) -> Self {
        Self {
            config,
            reader,
            state_store: Arc::new(Mutex::new(HashMap::new())),
            score_store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 执行单周期完整分析。
    pub async fn analyze_single(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
        time: Option<i64>,
    ) -> Result<AnalysisOutput, String> {
        let ic = &self.config.indicator;
        let sc = &self.config.state;
        let gc = &self.config.grid;
        let rc = &self.config.risk;
        let dqc = &self.config.data_quality;
        let interval_ms = crate::models::parse_interval_ms(interval).max(1);

        // 尽量取满 1000 根，满足 warmup / scoring / smoothing。
        let start_time = time.map(|t| t - interval_ms * 1200);
        let raw_klines = self
            .reader
            .fetch_klines(source, symbol, interval, start_time, time, Some(1000))
            .await
            .map_err(|e| format!("fetch klines failed: {e}"))?;

        if raw_klines.is_empty() {
            return Err("no klines data".into());
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let (klines, data_quality) =
            data_validator::validate_and_clean(&raw_klines, interval, now_ms, dqc);

        if klines.is_empty() {
            return Err("all klines invalid after cleaning".into());
        }

        // 严格只使用已闭合 K线推进评分和状态机。没有 closed kline 时直接 wait。
        let closed_klines: Vec<Kline> = klines.iter().filter(|k| k.is_closed).cloned().collect();
        if closed_klines.is_empty() {
            return Ok(build_wait_output(
                source,
                symbol,
                interval,
                &klines,
                &data_quality,
                &self.config,
                "no closed kline available; unclosed kline cannot advance state",
            ));
        }

        if closed_klines.len() < 60 {
            return Ok(build_wait_output(
                source,
                symbol,
                interval,
                &closed_klines,
                &data_quality,
                &self.config,
                "warmup 不满足",
            ));
        }

        let use_idx = closed_klines.len() - 1;
        let close: Vec<f64> = closed_klines.iter().map(|k| k.close).collect();
        let high: Vec<f64> = closed_klines.iter().map(|k| k.high).collect();
        let low: Vec<f64> = closed_klines.iter().map(|k| k.low).collect();
        let volume: Vec<f64> = closed_klines.iter().map(|k| k.volume).collect();

        let boll_result = boll::compute_boll(&close, ic.boll_period, ic.boll_mult);
        let macd_result = macd::compute_macd(&close, ic.macd_fast, ic.macd_slow, ic.macd_signal);
        let atr_result = atr::compute_atr(&high, &low, &close, ic.atr_period);
        let adx_result = adx::compute_adx(&high, &low, &close, &atr_result.atr, ic.adx_period);
        let rsi_result = rsi::compute_rsi(&close, ic.rsi_period);
        let vol_ratio_result = vol_ratio::compute_volume_ratio(&volume, ic.volume_ma_period);
        let ma_result = ma::compute_ma(&close, 5);

        let donchian_result = if self.config.features.enable_donchian {
            donchian::compute_donchian(&high, &low, ic.donchian_period)
        } else {
            donchian::DonchianResult {
                upper: vec![IndicatorValue::Unavailable("disabled".into()); closed_klines.len()],
                lower: vec![IndicatorValue::Unavailable("disabled".into()); closed_klines.len()],
                mid: vec![IndicatorValue::Unavailable("disabled".into()); closed_klines.len()],
            }
        };

        let mut unavailable_fields: Vec<String> = Vec::new();
        let mut ind_results = IndicatorResults {
            close: close.clone(),
            high: high.clone(),
            low: low.clone(),
            boll_upper: extract_vals(&boll_result.upper, "BOLL upper", &mut unavailable_fields),
            boll_mid: extract_vals(&boll_result.mid, "BOLL mid", &mut unavailable_fields),
            boll_lower: extract_vals(&boll_result.lower, "BOLL lower", &mut unavailable_fields),
            boll_bandwidth: extract_vals(&boll_result.bandwidth, "BOLL bandwidth", &mut unavailable_fields),
            percent_b: extract_vals(&boll_result.percent_b, "%B", &mut unavailable_fields),
            macd_dif: extract_vals(&macd_result.dif, "MACD DIF", &mut unavailable_fields),
            macd_dea: extract_vals(&macd_result.dea, "MACD DEA", &mut unavailable_fields),
            macd_hist: extract_vals(&macd_result.hist, "MACD hist", &mut unavailable_fields),
            macd_golden_cross: macd_result.golden_cross,
            macd_death_cross: macd_result.death_cross,
            atr: extract_vals(&atr_result.atr, "ATR", &mut unavailable_fields),
            adx: extract_vals(&adx_result.adx, "ADX", &mut unavailable_fields),
            plus_di: extract_vals(&adx_result.plus_di, "+DI", &mut unavailable_fields),
            minus_di: extract_vals(&adx_result.minus_di, "-DI", &mut unavailable_fields),
            rsi: extract_vals(&rsi_result, "RSI", &mut unavailable_fields),
            ma20: extract_vals(&ma_result.ma20, "MA20", &mut unavailable_fields),
            ma60: extract_vals(&ma_result.ma60, "MA60", &mut unavailable_fields),
            ema20: extract_vals(&ma_result.ema20, "EMA20", &mut unavailable_fields),
            ma_spread: extract_vals(&ma_result.ma_spread, "MA spread", &mut unavailable_fields),
            ma20_slope: extract_vals(&ma_result.ma20_slope, "MA20 slope", &mut unavailable_fields),
            ema20_deviation: extract_vals(&ma_result.ema20_deviation, "EMA20 dev", &mut unavailable_fields),
            volume_ratio: extract_vals(&vol_ratio_result, "Vol Ratio", &mut unavailable_fields),
            donchian_upper: extract_vals(&donchian_result.upper, "Donchian upper", &mut unavailable_fields),
            donchian_lower: extract_vals(&donchian_result.lower, "Donchian lower", &mut unavailable_fields),
            availability: IndicatorAvailability { ready: false, min_required_bars: 150, warmup_bars: 1000, unavailable_fields: vec![] },
        };

        let mut core_unavailable = Vec::new();
        for (name, ok) in [
            ("BOLL upper", finite_at(&ind_results.boll_upper, use_idx)),
            ("BOLL mid", finite_at(&ind_results.boll_mid, use_idx)),
            ("BOLL lower", finite_at(&ind_results.boll_lower, use_idx)),
            ("BOLL bandwidth", finite_at(&ind_results.boll_bandwidth, use_idx)),
            ("MACD hist", finite_at(&ind_results.macd_hist, use_idx)),
            ("ATR", finite_at(&ind_results.atr, use_idx)),
            ("ADX", finite_at(&ind_results.adx, use_idx)),
            ("+DI", finite_at(&ind_results.plus_di, use_idx)),
            ("-DI", finite_at(&ind_results.minus_di, use_idx)),
            ("RSI", finite_at(&ind_results.rsi, use_idx)),
            ("MA20", finite_at(&ind_results.ma20, use_idx)),
            ("MA60", finite_at(&ind_results.ma60, use_idx)),
            ("MA spread", finite_at(&ind_results.ma_spread, use_idx)),
            ("Volume Ratio", finite_at(&ind_results.volume_ratio, use_idx)),
        ] {
            if !ok { core_unavailable.push(name.to_string()); }
        }
        let indicator_ready = core_unavailable.is_empty();
        unavailable_fields.extend(core_unavailable.iter().map(|f| format!("core unavailable at last closed bar: {f}")));
        ind_results.availability = IndicatorAvailability {
            ready: indicator_ready,
            min_required_bars: 150,
            warmup_bars: 1000,
            unavailable_fields,
        };

        let (raw_scores, score_breakdown) = scoring::compute_raw_scores(
            &ind_results,
            use_idx,
            ic,
            sc,
            self.config.features.enable_score_conflict_adjustment,
        );

        let store_key = self.store_key(source, symbol, interval);
        let (smoothed_scores, prev_smoothed) = self.update_score_history(&store_key, raw_scores.clone());
        let momentum = if self.config.features.enable_score_momentum {
            scoring::score_momentum(&smoothed_scores, &prev_smoothed)
        } else {
            ScoreMomentum::default()
        };

        let mut ctx = {
            let mut states = self.state_store.lock().expect("state mutex poisoned");
            states.remove(&store_key).unwrap_or_default()
        };
        ctx.required_confirm_bars = sc.confirm_bars;

        let closed_klines_for_sm: Vec<(i64, f64, f64, f64, f64)> = closed_klines
            .iter()
            .map(|k| (k.open_time, k.open, k.high, k.low, k.close))
            .collect();

        let transition = state_machine::advance_state(
            &smoothed_scores,
            &data_quality,
            indicator_ready,
            &mut ctx,
            sc,
            self.config.features.enable_fake_breakout_filter,
            &closed_klines_for_sm,
        );

        {
            let mut states = self.state_store.lock().expect("state mutex poisoned");
            states.insert(store_key, ctx);
        }

        let state = transition.final_state;
        let state_phase = transition.final_state_phase;

        let risk_override = risk::evaluate_override(&data_quality, None, rc);
        let risk_decision = risk::build_risk_decision(
            state,
            risk_override,
            &data_quality,
            None,
            rc,
            self.config.features.enable_exchange_constraints,
        );

        let conf = confidence::compute_confidence(
            &raw_scores,
            &data_quality,
            &ind_results.availability,
            state,
            state_phase,
            1.0,
        );

        let display_grid = grid_plan::build_display_grid_plan(
            state,
            ind_results.boll_mid.get(use_idx).copied().filter(|v| v.is_finite()),
            ind_results.boll_upper.get(use_idx).copied().filter(|v| v.is_finite()),
            ind_results.boll_lower.get(use_idx).copied().filter(|v| v.is_finite()),
            ind_results.atr.get(use_idx).copied().filter(|v| v.is_finite()),
            conf.final_confidence,
            gc,
        );

        let signals = signal::generate_signals(
            &transition,
            &raw_scores,
            closed_klines[use_idx].close,
            closed_klines[use_idx].open_time,
        );

        Ok(output::build_analysis_output(
            source,
            symbol,
            interval,
            closed_klines[use_idx].open_time,
            true,
            &data_quality,
            &ind_results.availability,
            &raw_scores,
            &smoothed_scores,
            &momentum,
            &score_breakdown,
            state,
            state_phase,
            &transition,
            &conf,
            risk_override,
            &risk_decision,
            &display_grid,
            &signals,
            &self.config,
        ))
    }

    /// 执行多周期分析。以 lower timeframe 最新已闭合 K线 close_time 为 anchor，
    /// higher/middle 只能使用 close_time <= anchor 的快照。
    pub async fn analyze_multi_tf(
        &self,
        source: &str,
        symbol: &str,
    ) -> Result<MultiTfAnalysisOutput, String> {
        if !self.config.features.enable_multi_timeframe {
            return Err("multi_timeframe feature is disabled".into());
        }

        let tf_config = &self.config.multi_timeframe;
        let (higher_i, middle_i, lower_i) = tf_config.intervals();
        let middle_interval = middle_i.ok_or_else(|| "middle interval is required".to_string())?;
        let lower_interval = lower_i.unwrap_or(middle_interval);

        let lower_output = self.analyze_single(source, symbol, lower_interval, None).await?;
        let anchor_close_time = lower_output.time + crate::models::parse_interval_ms(lower_interval);

        let middle_output = if middle_interval == lower_interval {
            lower_output.clone()
        } else {
            self.analyze_single(source, symbol, middle_interval, Some(anchor_close_time)).await?
        };
        let higher_output = match higher_i {
            Some(higher_interval) => Some(self.analyze_single(source, symbol, higher_interval, Some(anchor_close_time)).await?),
            None => None,
        };

        let mut snapshots = Vec::new();
        if let Some(out) = higher_output.as_ref() {
            snapshots.push(snapshot_from_output(source, symbol, out));
        }
        snapshots.push(snapshot_from_output(source, symbol, &middle_output));
        if lower_interval != middle_interval {
            snapshots.push(snapshot_from_output(source, symbol, &lower_output));
        }

        let higher = if higher_output.is_some() { snapshots.first() } else { None };
        let middle_idx = if higher_output.is_some() { 1 } else { 0 };
        let middle = snapshots.get(middle_idx).ok_or_else(|| "missing middle snapshot".to_string())?;
        let lower = if lower_interval != middle_interval { snapshots.last() } else { None };

        let (merged_state, merged_phase, reasons) =
            multi_tf::merge_multi_timeframe(higher, middle, lower);

        let risk_decision = RiskDecision {
            risk_level: match merged_state {
                MarketState::DowntrendRisk => RiskLevel::HardBlock,
                MarketState::DownBreakWarning | MarketState::UptrendFollow => RiskLevel::SoftBlock,
                MarketState::RangeGrid => RiskLevel::Advisory,
                _ => RiskLevel::Advisory,
            },
            risk_override: RiskOverride::None,
            allowed_grid_modes: if merged_state == MarketState::RangeGrid {
                vec![AllowedGridMode::RangeGrid]
            } else if merged_state == MarketState::UptrendFollow {
                vec![AllowedGridMode::UptrendFollow]
            } else {
                vec![]
            },
            order_permission: if merged_state == MarketState::RangeGrid {
                OrderPermission::NewOrdersAllowed
            } else if merged_state == MarketState::DowntrendRisk {
                OrderPermission::ReduceOnly
            } else {
                OrderPermission::ReadOnly
            },
            position_action: if merged_state == MarketState::DowntrendRisk { PositionAction::ReduceByRatio } else { PositionAction::Hold },
            reduce_position_ratio: if merged_state == MarketState::DowntrendRisk { Some(self.config.risk.default_reduce_position_ratio) } else { None },
            reduce_reference: if merged_state == MarketState::DowntrendRisk { Some("total_position".into()) } else { None },
            require_manual_confirm: false,
            action_ttl_ms: 300_000,
            expire_at: chrono::Utc::now().timestamp_millis() + 300_000,
            reasons: reasons.clone(),
        };

        let display_grid = DisplayGridPlan {
            enabled: merged_state == MarketState::RangeGrid,
            mode: if merged_state == MarketState::RangeGrid { GridMode::RangeGrid } else { GridMode::Wait },
            boundary_mode: "boll".into(),
            lower: None,
            upper: None,
            center: None,
            grid_count: if merged_state == MarketState::RangeGrid { self.config.grid.grid_count } else { 0 },
            grid_step: None,
            risk_level: risk_decision.risk_level,
            confidence: snapshots.iter().map(|s| s.confidence).fold(1.0, f64::min),
        };

        Ok(output::build_multi_tf_output(
            source,
            symbol,
            snapshots,
            merged_state,
            merged_phase,
            &risk_decision,
            &display_grid,
            reasons,
        ))
    }

    fn store_key(&self, source: &str, symbol: &str, interval: &str) -> String {
        format!("{}:{}:{}:{}", source, symbol, interval, self.config.config_hash())
    }

    fn update_score_history(&self, key: &str, raw: Scores) -> (Scores, Scores) {
        let mut store = self.score_store.lock().expect("score mutex poisoned");
        let history = store.entry(key.to_string()).or_default();
        history.push(raw);
        if history.len() > self.config.indicator.percentile_window.max(100) {
            let drain_len = history.len() - self.config.indicator.percentile_window.max(100);
            history.drain(0..drain_len);
        }
        let smoothed = scoring::smooth_scores(history, self.config.indicator.score_smooth_period);
        let current = smoothed.last().cloned().unwrap_or_default();
        let previous = if smoothed.len() >= 2 { smoothed[smoothed.len() - 2].clone() } else { current.clone() };
        (current, previous)
    }
}

fn snapshot_from_output(source: &str, symbol: &str, output: &AnalysisOutput) -> TimeframeSnapshotRef {
    TimeframeSnapshotRef {
        source: source.into(),
        symbol: symbol.into(),
        interval: output.interval.clone(),
        open_time: output.time,
        close_time: output.time + crate::models::parse_interval_ms(&output.interval),
        is_closed: output.is_closed_kline,
        state: output.state,
        state_phase: output.state_phase,
        candidate_bars: output.state_transition.candidate_bars,
        required_confirm_bars: output.state_transition.candidate_bars.max(1),
        cooldown_remaining_bars: output.state_transition.cooldown_remaining_bars,
        raw_scores: output.raw_scores.clone(),
        smoothed_scores: output.smoothed_scores.clone(),
        confidence: output.confidence_breakdown.final_confidence,
    }
}

fn finite_at(v: &[f64], idx: usize) -> bool {
    v.get(idx).copied().map(|x| x.is_finite()).unwrap_or(false)
}

/// 从 IndicatorValue 序列提取 f64 值，不可用的标记为 NaN 并记录原因。
fn extract_vals(values: &[IndicatorValue], name: &str, unavailable_fields: &mut Vec<String>) -> Vec<f64> {
    let mut result = Vec::with_capacity(values.len());
    for (i, v) in values.iter().enumerate() {
        match v {
            IndicatorValue::Available(val) => result.push(*val),
            IndicatorValue::Unavailable(reason) => {
                result.push(f64::NAN);
                unavailable_fields.push(format!("{name}[{i}]: {reason}"));
            }
        }
    }
    result
}

fn build_wait_output(
    source: &str,
    symbol: &str,
    interval: &str,
    klines: &[Kline],
    dq: &DataQuality,
    config: &KlinesToolsConfig,
    reason: &str,
) -> AnalysisOutput {
    let last = klines.last().expect("build_wait_output requires at least one kline");
    let now_ms = chrono::Utc::now().timestamp_millis();
    AnalysisOutput {
        schema_version: "1.2".into(),
        model_version: "rule-v1".into(),
        config_version: "grid-analysis-v1.0.3".into(),
        config_hash: Some(config.config_hash()),
        enabled_features: config.enabled_features(),
        source: source.into(),
        symbol: symbol.into(),
        interval: interval.into(),
        time: last.open_time,
        generated_at: now_ms,
        is_closed_kline: last.is_closed,
        data_quality: dq.clone(),
        indicator_availability: IndicatorAvailability {
            ready: false,
            min_required_bars: 150,
            warmup_bars: 1000,
            unavailable_fields: vec![reason.into()],
        },
        raw_scores: Scores::default(),
        smoothed_scores: Scores::default(),
        score_momentum: ScoreMomentum::default(),
        score_breakdown: ScoreBreakdown { range: vec![], up: vec![], down: vec![] },
        state: MarketState::Wait,
        state_phase: StatePhase::Observing,
        state_transition: StateTransition {
            previous_state: MarketState::Wait,
            candidate_state: None,
            final_state: MarketState::Wait,
            final_state_phase: StatePhase::Observing,
            transition_type: "insufficient_or_unclosed_data".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: 0,
            reasons: vec![reason.into()],
        },
        confidence_breakdown: ConfidenceBreakdown {
            state_evidence: 0.0,
            data_quality: dq.quality_score,
            indicator_availability: 0.0,
            timeframe_alignment: 1.0,
            state_stability: 0.0,
            final_confidence: 0.0,
        },
        risk_override: RiskOverride::DataQualityBlock,
        risk_decision: RiskDecision {
            risk_level: RiskLevel::HardBlock,
            risk_override: RiskOverride::DataQualityBlock,
            allowed_grid_modes: vec![],
            order_permission: OrderPermission::None,
            position_action: PositionAction::Hold,
            reduce_position_ratio: None,
            reduce_reference: None,
            require_manual_confirm: false,
            action_ttl_ms: 300_000,
            expire_at: now_ms + 300_000,
            reasons: vec![reason.into()],
        },
        grid_plan: DisplayGridPlan {
            enabled: false,
            mode: GridMode::Wait,
            boundary_mode: "boll".into(),
            lower: None,
            upper: None,
            center: None,
            grid_count: 0,
            grid_step: None,
            risk_level: RiskLevel::HardBlock,
            confidence: 0.0,
        },
        signals: vec![],
    }
}
