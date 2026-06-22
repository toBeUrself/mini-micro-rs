//! 主分析编排器：串联数据校验 → 指标计算 → 评分 → 状态机 → 风险 → 输出。

use indicators::{self, boll, macd, atr, adx, rsi, vol_ratio, ma, donchian};
use indicators::types::IndicatorValue;
use crate::{
    config::KlinesToolsConfig,
    data_validator, kline_reader::KlineReader,
    models::*,
    scoring, state_machine, risk, confidence, grid_plan, signal, output, multi_tf,
};

/// 分析器：持有配置和 K 线读取器。
#[derive(Clone)]
pub struct Analyzer {
    pub config: KlinesToolsConfig,
    pub reader: KlineReader,
}

impl Analyzer {
    /// 创建分析器。
    pub fn new(config: KlinesToolsConfig, reader: KlineReader) -> Self {
        Self { config, reader }
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

        // 1. 拉取 K 线（尽量多取，用于指标 warmup）
        let raw_klines = self
            .reader
            .fetch_klines(source, symbol, interval, time.map(|t| t - 1000 * 3600 * 24 * 7), time, Some(500))
            .await
            .map_err(|e| format!("fetch klines failed: {e}"))?;

        if raw_klines.is_empty() {
            return Err("no klines data".into());
        }

        // 2. 数据校验与清洗
        let now_ms = chrono::Utc::now().timestamp_millis();
        let (klines, data_quality) =
            data_validator::validate_and_clean(&raw_klines, interval, now_ms, dqc);

        if klines.is_empty() {
            return Err("all klines invalid after cleaning".into());
        }

        // 3. 确定使用的 K 线：已闭合 K 线用于状态确认，未闭合只能观察
        let closed_indices: Vec<usize> = klines
            .iter()
            .enumerate()
            .filter(|(_, k)| k.is_closed)
            .map(|(i, _)| i)
            .collect();

        let last_closed_idx = closed_indices.last().copied();
        let last_idx = klines.len() - 1;
        let use_idx = last_closed_idx.unwrap_or(last_idx);
        let is_using_closed = last_closed_idx.is_some();

        // 如果数据不足，快速返回 wait
        if klines.len() < 60 {
            return Ok(build_wait_output(
                source, symbol, interval, &klines, &data_quality, &self.config, "warmup 不满足",
            ));
        }

        // 4. 提取 OHLCV 数组用于指标计算
        let close: Vec<f64> = klines.iter().map(|k| k.close).collect();
        let high: Vec<f64> = klines.iter().map(|k| k.high).collect();
        let low: Vec<f64> = klines.iter().map(|k| k.low).collect();
        let volume: Vec<f64> = klines.iter().map(|k| k.volume).collect();

        // 5. 指标计算
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
                upper: vec![IndicatorValue::Unavailable("disabled".into()); klines.len()],
                lower: vec![IndicatorValue::Unavailable("disabled".into()); klines.len()],
                mid: vec![IndicatorValue::Unavailable("disabled".into()); klines.len()],
            }
        };

        // 6. 收集指标到中间结构
        let mut unavailable_fields: Vec<String> = Vec::new();

        let ind_results = IndicatorResults {
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
            availability: IndicatorAvailability {
                ready: macd_result.hist.get(use_idx).and_then(|h| h.value()).is_some()
                    && atr_result.atr.get(use_idx).and_then(|a| a.value()).is_some(),
                min_required_bars: 150,
                warmup_bars: 1000,
                unavailable_fields: unavailable_fields.clone(),
            },
        };

        let indicator_ready = ind_results.availability.ready;

        // 7. 评分
        let (raw_scores, score_breakdown) = scoring::compute_raw_scores(
            &ind_results,
            use_idx,
            ic,
            sc,
            self.config.features.enable_score_conflict_adjustment,
        );

        // 平滑评分：在整个已闭合 K 线序列上计算
        // For now, we compute a single snapshot
        let smoothed_scores = raw_scores.clone();
        let prev_scores = Scores::default(); // simplified
        let momentum = scoring::score_momentum(&raw_scores, &prev_scores);

        // 8. 状态机
        let mut ctx = state_machine::new_state_context();
        let closed_klines_for_sm: Vec<(i64, f64, f64, f64, f64)> = klines
            .iter()
            .filter(|k| k.is_closed)
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

        let state = transition.final_state;
        let state_phase = transition.final_state_phase;

        // 9. RiskOverride 评估
        let risk_override = risk::evaluate_override(&data_quality, None, rc);

        // 10. RiskDecision
        let risk_decision = risk::build_risk_decision(
            state,
            risk_override,
            &data_quality,
            None,
            rc,
            self.config.features.enable_exchange_constraints,
        );

        // 11. Confidence
        let conf = confidence::compute_confidence(
            &raw_scores,
            &data_quality,
            &ind_results.availability,
            state,
            state_phase,
            1.0, // single timeframe
        );

        // 12. GridPlan
        let display_grid = grid_plan::build_display_grid_plan(
            state,
            ind_results.boll_mid.get(use_idx).copied().filter(|v| v.is_finite()),
            ind_results.boll_upper.get(use_idx).copied().filter(|v| v.is_finite()),
            ind_results.boll_lower.get(use_idx).copied().filter(|v| v.is_finite()),
            ind_results.atr.get(use_idx).copied().filter(|v| v.is_finite()),
            conf.final_confidence,
            gc,
        );

        // 13. Signals
        let signals = signal::generate_signals(
            &transition,
            &raw_scores,
            klines[use_idx].close,
            klines[use_idx].open_time,
        );

        // 14. 构建输出
        let analysis_output = output::build_analysis_output(
            source,
            symbol,
            interval,
            klines[use_idx].open_time,
            is_using_closed,
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
        );

        Ok(analysis_output)
    }

    /// 执行多周期分析。
    pub async fn analyze_multi_tf(
        &self,
        source: &str,
        symbol: &str,
    ) -> Result<MultiTfAnalysisOutput, String> {
        if !self.config.features.enable_multi_timeframe {
            return Err("multi_timeframe feature is disabled".into());
        }

        let tf_config = &self.config.multi_timeframe;
        let intervals = tf_config.intervals();

        let mut snapshots = Vec::new();

        // 拉取各周期数据
        for interval_opt in [intervals.0, intervals.1, intervals.2].iter() {
            if let Some(interval) = interval_opt {
                match self.analyze_single(source, symbol, interval, None).await {
                    Ok(output) => {
                        snapshots.push(TimeframeSnapshotRef {
                            source: source.into(),
                            symbol: symbol.into(),
                            interval: interval.to_string(),
                            open_time: output.time,
                            close_time: output.time + crate::models::parse_interval_ms(interval),
                            is_closed: output.is_closed_kline,
                            state: output.state,
                            state_phase: output.state_phase,
                            candidate_bars: output.state_transition.candidate_bars,
                            required_confirm_bars: self.config.state.confirm_bars,
                            cooldown_remaining_bars: output.state_transition.cooldown_remaining_bars,
                            raw_scores: output.raw_scores.clone(),
                            smoothed_scores: output.smoothed_scores.clone(),
                            confidence: output.confidence_breakdown.final_confidence,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch {interval}: {e}");
                    }
                }
            }
        }

        // 找到 middle snapshot（索引 1）
        if snapshots.len() < 2 {
            return Err("insufficient timeframe data".into());
        }

        let middle = &snapshots[1];
        let higher = if snapshots.len() > 2 { Some(&snapshots[0]) } else { None };
        let lower = if snapshots.len() > 2 { Some(&snapshots[2]) } else { None };

        let (merged_state, merged_phase, reasons) =
            multi_tf::merge_multi_timeframe(higher, middle, lower);

        // 构建合并后的风险决策
        let _rc = &self.config.risk;
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
            position_action: PositionAction::Hold,
            reduce_position_ratio: None,
            reduce_reference: None,
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
            grid_count: 20,
            grid_step: None,
            risk_level: RiskLevel::Advisory,
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
}

/// 从 IndicatorValue 序列提取 f64 值，不可用的标记为 NaN 并记录原因。
fn extract_vals(
    values: &[IndicatorValue],
    name: &str,
    unavailable_fields: &mut Vec<String>,
) -> Vec<f64> {
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

/// 构建 wait 状态的快速返回输出。
fn build_wait_output(
    source: &str,
    symbol: &str,
    interval: &str,
    klines: &[Kline],
    dq: &DataQuality,
    config: &KlinesToolsConfig,
    reason: &str,
) -> AnalysisOutput {
    let last = klines.last().unwrap();
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
            transition_type: "insufficient_data".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: 0,
            reasons: vec![reason.into()],
        },
        confidence_breakdown: ConfidenceBreakdown {
            state_evidence: 0.0, data_quality: dq.quality_score,
            indicator_availability: 0.0, timeframe_alignment: 1.0,
            state_stability: 0.0, final_confidence: 0.0,
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
            enabled: false, mode: GridMode::Wait, boundary_mode: "boll".into(),
            lower: None, upper: None, center: None,
            grid_count: 0, grid_step: None,
            risk_level: RiskLevel::HardBlock, confidence: 0.0,
        },
        signals: vec![],
    }
}
