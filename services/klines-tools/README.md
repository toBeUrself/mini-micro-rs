# klines-tools

K 线指标分析与网格状态识别服务。

将 K 线数据转化为可解释、可回测、可展示、可被风控系统消费的行情状态。

## 核心能力

- 8 个核心指标计算（BOLL/MACD/ATR/ADX/MA/RSI/Volume Ratio/Donchian）
- 三维评分系统（range_score / up_score / down_score，0~100）
- 六状态状态机（Wait / RangeGrid / UpBreakWarning / UptrendFollow / DownBreakWarning / DowntrendRisk）
- 假突破过滤 + 评分互斥修正 + 冷却期
- 风险覆盖层（RiskOverride）+ 可执行风险决策（RiskDecision）
- 网格计划（DisplayGridPlan + ExecutableGridPlan + GridLevel）
- 多周期时间对齐与合并决策
- JSON 输出契约（schema_version / config_hash / enabled_features）
- 16 个 Feature Flag 控制增强功能

## API

所有接口均为只读分析，不涉及下单。

```
GET /health
GET /api/v1/tools/analysis/market-state?symbol=BTCUSDT&interval=5m[&source=binance][&time=1710000000000]
GET /api/v1/tools/analysis/grid-plan?symbol=BTCUSDT&interval=5m[&source=binance]
GET /api/v1/tools/analysis/signals?symbol=BTCUSDT&interval=5m[&source=binance]
GET /api/v1/tools/analysis/multi-timeframe-state?symbol=BTCUSDT[&source=binance]
```

### 参数

| 参数 | 必填 | 说明 |
|---|---|---|
| `symbol` | 是 | 交易对，如 `BTCUSDT` |
| `interval` | 是（单周期） | K 线周期，如 `1m`/`5m`/`15m`/`30m`/`1h`/`4h` |
| `source` | 否 | 数据源，默认 `binance` |
| `time` | 否 | 指定 K 线时间戳（毫秒），不传使用最新 |

### market-state 响应示例

```json
{
  "schema_version": "1.2",
  "model_version": "rule-v1",
  "config_version": "grid-analysis-v1.0.3",
  "config_hash": "sha256:...",
  "enabled_features": ["score_conflict_adjustment"],
  "source": "binance",
  "symbol": "BTCUSDT",
  "interval": "5m",
  "state": "range_grid",
  "state_phase": "confirmed",
  "risk_override": "none",
  "raw_scores": { "range_score": 74, "up_score": 20, "down_score": 12 },
  "smoothed_scores": { "range_score": 72, "up_score": 18, "down_score": 10 },
  "confidence_breakdown": { "final_confidence": 0.72 },
  "risk_decision": {
    "risk_level": "advisory",
    "order_permission": "new_orders_allowed",
    "position_action": "hold"
  },
  "grid_plan": {
    "enabled": true,
    "mode": "range_grid",
    "boundary_mode": "boll",
    "lower": 67360.0,
    "upper": 68600.0,
    "grid_count": 20
  }
}
```

## 配置

```toml
bind = "127.0.0.1:8081"
app_api_base_url = "http://127.0.0.1:8080"

[features]
enable_multi_timeframe = false
enable_donchian = false
enable_fake_breakout_filter = false
# ... 共 16 个 feature flags

[indicator]
boll_period = 20
boll_mult = 2.0
# ...

[state]
range_enter = 65
range_exit = 55
confirm_bars = 3
# ...

[grid]
grid_count = 20
boundary_mode = "boll"
# ...
```

完整配置参考 `klines-tools.example.toml`。

## 状态机

```
                ┌──────────┐
        ┌──────→│   Wait   │←──────┐
        │       └────┬─────┘       │
        │            │ range≥65    │ 数据不足/冲突
        │       ┌────▼─────┐       │
        │       │RangeGrid │       │
        │       └──┬───┬───┘       │
        │   up≥55  │   │  down≥55  │
        │  ┌───────┘   └───────┐  │
   ┌────▼─────┐          ┌────▼─────┐
   │UpBreak   │          │DownBreak │
   │Warning   │          │Warning   │
   └────┬─────┘          └────┬─────┘
        │ up≥80              │ down≥80
   ┌────▼─────┐          ┌────▼─────┐
   │Uptrend   │          │Downtrend │
   │Follow    │          │Risk      │
   └──────────┘          └──────────┘
```

- 候选状态需连续 `confirm_bars`（默认3）根闭合 K 线确认
- 退出网格后有冷却期（默认5根），防止频繁切换
- 止损后使用更长冷却期（默认20根）

## 风险分层

```
MarketState   → 只表达行情状态（震荡/上涨/下跌）
RiskOverride  → 数据/账户/人工/交易所覆盖层
RiskDecision  → 可执行动作契约（允许模式/订单权限/仓位动作）
```

四层概念禁止混用。`MarketState` 永远不会被 `GlobalHardStop` 覆盖，后者只体现在 `RiskDecision` 中。

## 启动

```bash
cargo run -p klines-tools -- klines-tools.toml
```

依赖 `app-api` 在配置的 `app_api_base_url` 提供 K 线数据。

## 数据流

```
app-api (HTTP)
    ↓
KlineReader (拉取 K 线)
    ↓
DataValidator (校验/排序/去重/缺失检测)
    ↓
Indicators (BOLL/MACD/ATR/ADX/MA/RSI/VolRatio/Donchian)
    ↓
Scoring (三维评分 + 平滑 + 动能 + 互斥修正)
    ↓
StateMachine (6状态 + StatePhase + 假突破过滤)
    ↓
RiskDecision + GridPlan + Signals
    ↓
JSON Output (schema_version + config_hash)
```

## Feature Flags

| Flag | 阶段 | 默认 | 说明 |
|---|---|---|---|
| `enable_multi_timeframe` | Phase 3 | false | 多周期合并 |
| `enable_donchian` | Phase 2 | false | Donchian Channel |
| `enable_percent_b` | Phase 2 | true | BOLL %B |
| `enable_ema20_deviation` | Phase 2 | true | EMA20 偏离率 |
| `enable_score_momentum` | Phase 2 | false | 评分动能 |
| `enable_score_conflict_adjustment` | Phase 2 | true | 评分互斥修正 |
| `enable_fake_breakout_filter` | Phase 2 | false | 假突破过滤 |
| `enable_exchange_constraints` | Phase 3 | false | 交易所约束 |
| 其余 | Phase 4 | false | 后置增强 |

## 测试

```bash
cargo test -p klines-tools
```

## 相关文档

- `指标分析docs/SPEC.zh-CN.md` — 完整需求与系统设计
- `指标分析docs/IMPLEMENTATION_PLAN.zh-CN.md` — 分阶段实施计划
