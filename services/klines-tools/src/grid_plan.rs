//! 网格计划生成：DisplayGridPlan + GridLevel + 交易所约束。

use rust_decimal::Decimal;
use crate::config::GridConfig;
use crate::models::{
    MarketState, GridMode, DisplayGridPlan, OrderSide, GridLevel, ExecutableGridPlan,
    ExchangeConstraints, RiskLevel,
};

/// 从 BOLL 指标和状态生成展示网格计划。
pub fn build_display_grid_plan(
    state: MarketState,
    boll_mid: Option<f64>,
    boll_upper: Option<f64>,
    boll_lower: Option<f64>,
    atr: Option<f64>,
    confidence: f64,
    gc: &GridConfig,
) -> DisplayGridPlan {
    let (enabled, mode) = match state {
        MarketState::RangeGrid => (true, GridMode::RangeGrid),
        MarketState::UpBreakWarning => (true, GridMode::RangeGrid),
        MarketState::UptrendFollow => (true, GridMode::UptrendFollow),
        MarketState::Wait | MarketState::DownBreakWarning | MarketState::DowntrendRisk => {
            (false, GridMode::Wait)
        }
    };

    let center = boll_mid;
    let upper = boll_upper;
    let lower = boll_lower;

    // 使用 BOLL 作为边界
    let (boundary_mode, planned_lower, planned_upper) = match gc.boundary_mode.as_str() {
        "boll" => (gc.boundary_mode.clone(), lower, upper),
        "donchian" => (gc.boundary_mode.clone(), lower, upper), // same structure
        _ => ("boll".into(), lower, upper),
    };

    let grid_step = match (planned_lower, planned_upper) {
        (Some(l), Some(u)) if enabled => {
            let width = u - l;
            Some(width / gc.grid_count as f64)
        }
        _ => None,
    };

    // 限制网格宽度
    if let (Some(c), Some(l), Some(u)) = (center, planned_lower, planned_upper) {
        let width_pct = if c > 0.0 { (u - l) / c } else { 0.0 };
        if width_pct > gc.max_grid_width_by_percent {
            tracing::warn!(
                "grid width {:.1}% exceeds max {:.1}%, confidence will reflect this",
                width_pct * 100.0,
                gc.max_grid_width_by_percent * 100.0
            );
        }
        if let Some(atr_val) = atr {
            if (u - l) / atr_val > gc.max_grid_width_by_atr {
                tracing::warn!(
                    "grid width {:.1}x ATR exceeds max {:.1}x",
                    (u - l) / atr_val,
                    gc.max_grid_width_by_atr
                );
            }
        }
    }

    DisplayGridPlan {
        enabled,
        mode,
        boundary_mode,
        lower: planned_lower,
        upper: planned_upper,
        center,
        grid_count: gc.grid_count,
        grid_step,
        risk_level: if enabled { RiskLevel::Advisory } else { RiskLevel::SoftBlock },
        confidence,
    }
}

/// 生成可执行网格计划（含 GridLevel）。
///
/// - `capital_per_level`：每层分配的资金（计价币），用于计算 qty。不传则使用最小单位。
pub fn build_executable_grid_plan(
    display: &DisplayGridPlan,
    constraints: Option<&ExchangeConstraints>,
    capital_per_level: Option<Decimal>,
) -> ExecutableGridPlan {
    let Some(lower) = display.lower else {
        return ExecutableGridPlan {
            enabled: false,
            mode: display.mode,
            levels: vec![],
            total_required_capital: Decimal::ZERO,
            executable_level_count: 0,
        };
    };
    let Some(_upper) = display.upper else {
        return ExecutableGridPlan {
            enabled: false,
            mode: display.mode,
            levels: vec![],
            total_required_capital: Decimal::ZERO,
            executable_level_count: 0,
        };
    };
    let Some(step) = display.grid_step else {
        return ExecutableGridPlan {
            enabled: false,
            mode: display.mode,
            levels: vec![],
            total_required_capital: Decimal::ZERO,
            executable_level_count: 0,
        };
    };

    let mut levels = Vec::new();
    let mut total_capital = Decimal::ZERO;
    let mut executable_count = 0;

    // 计算每层数量：如果有 capital_per_level，用它除以买价
    let default_qty = capital_per_level.unwrap_or(Decimal::new(1, 3)); // 默认 0.001

    for i in 0..display.grid_count {
        let buy_price = lower + step * i as f64;
        let sell_price = buy_price + step;

        // 动态 qty: capital / price
        let qty = if let Some(&cp) = capital_per_level.as_ref() {
            let price_dec = Decimal::try_from(buy_price).unwrap_or(Decimal::ONE);
            if price_dec > Decimal::ZERO { cp / price_dec } else { default_qty }
        } else {
            default_qty
        };

        let (buy_level, buy_exec) = make_grid_level(
            OrderSide::Buy, buy_price, qty, constraints,
        );
        let (sell_level, sell_exec) = make_grid_level(
            OrderSide::Sell, sell_price, qty, constraints,
        );

        if buy_exec { executable_count += 1; }
        if sell_exec { executable_count += 1; }

        total_capital += buy_level.notional;

        levels.push(buy_level);
        levels.push(sell_level);
    }

    ExecutableGridPlan {
        enabled: display.enabled,
        mode: display.mode,
        levels,
        total_required_capital: total_capital,
        executable_level_count: executable_count,
    }
}

/// 创建单个网格层，应用交易所约束。
fn make_grid_level(
    side: OrderSide,
    raw_price: f64,
    raw_qty: Decimal,
    constraints: Option<&ExchangeConstraints>,
) -> (GridLevel, bool) {
    let mut executable = true;
    let mut disabled_reason: Option<String> = None;
    let raw_price_dec = Decimal::try_from(raw_price).unwrap_or(Decimal::ZERO);

    let (price, qty) = if let Some(c) = constraints {
        let tick = c.tick_size;
        let rounded_price = (raw_price_dec / tick).round() * tick;
        let rounded_price = rounded_price.round_dp(c.price_precision);

        let step = c.step_size;
        let rounded_qty = (raw_qty / step).round() * step;
        let rounded_qty = rounded_qty.round_dp(c.quantity_precision);

        if rounded_qty < c.min_qty {
            executable = false;
            disabled_reason = Some(format!("qty below min_qty: {rounded_qty}"));
        }
        let notional = rounded_price * rounded_qty;
        if notional < c.min_notional {
            executable = false;
            disabled_reason = Some(format!("notional below min: {notional}"));
        }

        (rounded_price, rounded_qty)
    } else {
        (raw_price_dec, raw_qty)
    };

    let notional = price * qty;

    (
        GridLevel {
            side,
            raw_price: raw_price_dec,
            price,
            raw_qty,
            qty,
            notional,
            executable,
            disabled_reason,
        },
        executable,
    )
}
