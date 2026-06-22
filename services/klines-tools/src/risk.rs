//! 风险决策层：RiskOverride → RiskDecision。
//!
//! 市场状态只表达行情，RiskOverride 和责任动作不混入 MarketState。
//!
//! 职责分层（SPEC 14）：
//! - `MarketRiskDecision`：由 K线/指标/状态机产生，只判断市场风险。本模块的主要输出。
//! - `PortfolioRiskDecision`：由外部账户风控层计算（止损/减仓/emergency_stop）。
//!   本模块可消费或透传外部 risk_override，但不得凭 K线自行判断账户硬止损。

use rust_decimal::Decimal;
use crate::config::RiskConfig;
use crate::models::{
    MarketState, RiskOverride, RiskLevel, RiskDecision, AllowedGridMode,
    OrderPermission, PositionAction, DataQuality, PortfolioRiskInput, MarketType,
};

/// 从 RiskOverride 生成执行层 RiskDecision。
///
/// - `market_type`：区分 spot/futures 语义（SPEC 13.4）
/// - `enable_exchange_constraints`：是否启用交易所约束检查
pub fn build_risk_decision(
    market_state: MarketState,
    risk_override: RiskOverride,
    data_quality: &DataQuality,
    portfolio: Option<&PortfolioRiskInput>,
    rc: &RiskConfig,
    enable_exchange_constraints: bool,
    market_type: MarketType,
) -> RiskDecision {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let ttl = 300_000; // 5 min
    let mut reasons: Vec<String> = Vec::new();

    // 如果启用交易所约束但未提供验证通过标识，则该 override 由外部设置
    if enable_exchange_constraints {
        // ExchangeConstraintBlock 应由上游在验证交易所参数后设置
        // 此处仅为透传，不做自行判断
    }

    // P0: RiskOverride 优先
    let (risk_level, order_perm, pos_action, allowed_modes) = match risk_override {
        RiskOverride::GlobalHardStop => {
            // 全局硬止损由外部风控层设置，本模块只透传到 RiskDecision
            // SPEC 14: 不改变 MarketState 语义
            reasons.push("外部全局硬止损触发".into());
            (
                RiskLevel::EmergencyStop,
                OrderPermission::None,
                PositionAction::StopLoss,
                vec![],
            )
        }
        RiskOverride::DataQualityBlock => {
            reasons.push("数据质量阻断".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReadOnly,
                PositionAction::Hold,
                vec![],
            )
        }
        RiskOverride::ManualBlock => {
            reasons.push("人工阻断".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReadOnly,
                PositionAction::ManualReview,
                vec![],
            )
        }
        RiskOverride::IndicatorUnavailableBlock => {
            reasons.push("核心指标不可用".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReadOnly,
                PositionAction::Hold,
                vec![],
            )
        }
        RiskOverride::ExchangeConstraintBlock => {
            reasons.push("交易所约束不满足".into());
            (
                RiskLevel::SoftBlock,
                OrderPermission::ReadOnly,
                PositionAction::Hold,
                vec![],
            )
        }
        RiskOverride::None => {
            // 从 MarketState 推导默认动作
            default_risk_from_state(market_state, data_quality, portfolio, rc, &mut reasons, market_type)
        }
    };

    let reduce_ratio: Option<Decimal> = if matches!(pos_action, PositionAction::ReduceByRatio) {
        Decimal::try_from(rc.default_reduce_position_ratio).ok()
    } else {
        None
    };

    RiskDecision {
        risk_level,
        risk_override,
        allowed_grid_modes: allowed_modes,
        order_permission: order_perm,
        position_action: pos_action,
        reduce_position_ratio: reduce_ratio,
        reduce_reference: if reduce_ratio.is_some() {
            Some("total_position".into())
        } else {
            None
        },
        require_manual_confirm: matches!(risk_level, RiskLevel::EmergencyStop | RiskLevel::HardBlock),
        action_ttl_ms: ttl,
        expire_at: now_ms + ttl,
        reasons,
    }
}

/// 从 MarketState 推导默认风险决策。
#[allow(clippy::too_many_arguments)]
fn default_risk_from_state(
    state: MarketState,
    dq: &DataQuality,
    portfolio: Option<&PortfolioRiskInput>,
    rc: &RiskConfig,
    reasons: &mut Vec<String>,
    market_type: MarketType,
) -> (RiskLevel, OrderPermission, PositionAction, Vec<AllowedGridMode>) {
    match state {
        MarketState::Wait => {
            reasons.push("方向不清晰或等待中".into());
            (RiskLevel::Advisory, OrderPermission::ReadOnly, PositionAction::Hold, vec![])
        }
        MarketState::RangeGrid => {
            if dq.quality_score < 0.7 {
                reasons.push(format!("数据质量过低({:.2})，不允许新开网格", dq.quality_score));
                return (RiskLevel::SoftBlock, OrderPermission::ReadOnly, PositionAction::Hold, vec![]);
            }
            // 消费外部 PortfolioRiskInput，但不自行判断 GlobalHardStop
            // 外部已经通过 RiskOverride 传入了全局硬止损判断
            if let Some(pf) = portfolio {
                if let Some(dd) = pf.max_equity_drawdown {
                    if dd > rc.max_drawdown {
                        reasons.push(format!("账户回撤({:.2}%)超过阈值，建议减仓", dd * 100.0));
                        // 注意：这里降级为 SoftBlock + ReduceOnly，不触发 EmergencyStop
                        // EmergencyStop 必须由外部风控层通过 RiskOverride::GlobalHardStop 传入
                        return (RiskLevel::SoftBlock, OrderPermission::ReduceOnly, PositionAction::ReduceByRatio, vec![]);
                    }
                }
            }
            reasons.push("震荡条件成立，允许普通震荡网格".into());
            (
                RiskLevel::Advisory,
                OrderPermission::NewOrdersAllowed,
                PositionAction::Hold,
                vec![AllowedGridMode::RangeGrid],
            )
        }
        MarketState::UpBreakWarning => {
            reasons.push("上涨突破预警，减少卖出".into());
            (
                RiskLevel::Advisory,
                OrderPermission::ReplaceOnly,
                PositionAction::Hold,
                vec![AllowedGridMode::RangeGrid],
            )
        }
        MarketState::UptrendFollow => {
            reasons.push("上涨趋势确认，关闭普通网格，允许趋势跟随".into());
            (
                RiskLevel::SoftBlock,
                OrderPermission::ReplaceOnly,
                PositionAction::CloseGridOnly,
                vec![AllowedGridMode::UptrendFollow],
            )
        }
        MarketState::DownBreakWarning => {
            reasons.push("下跌破位预警，暂停新增买单".into());
            (
                RiskLevel::SoftBlock,
                OrderPermission::ReplaceOnly,
                PositionAction::Hold,
                vec![AllowedGridMode::RangeGrid],
            )
        }
        MarketState::DowntrendRisk => {
            reasons.push("下跌风险确认，禁止新开多头网格".into());
            // spot vs futures: ReduceOnly 在 spot 下解释为"不增加净多仓"
            match market_type {
                MarketType::Spot => {
                    reasons.push("Spot市场：ReduceOnly 解释为不增加净多仓".into());
                }
                MarketType::UsdMarginedFutures | MarketType::CoinMarginedFutures => {
                    reasons.push("合约市场：使用交易所 reduce-only flag".into());
                }
            }
            (
                RiskLevel::HardBlock,
                OrderPermission::ReduceOnly,
                PositionAction::ReduceByRatio,
                vec![],
            )
        }
    }
}

/// 评估 RiskOverride 优先级。
///
/// SPEC 14.2：本模块只能输出 MarketRiskDecision。
/// 账户级 GlobalHardStop 必须由外部风控层通过 RiskOverride 参数传入。
/// 此函数只评估数据质量和指标可用性层面的 override。
pub fn evaluate_override(
    data_quality: &DataQuality,
    indicator_ready: bool,
) -> RiskOverride {
    // 数据质量严重不足
    if !data_quality.warmup_satisfied || data_quality.quality_score < 0.3 {
        return RiskOverride::DataQualityBlock;
    }

    // 核心指标不可用
    if !indicator_ready {
        return RiskOverride::IndicatorUnavailableBlock;
    }

    // 账户级硬止损不在此处判断，由外部 PortfolioRiskDecision 层负责。
    RiskOverride::None
}
