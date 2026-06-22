//! StateContext 跨请求持久化存储。
//!
//! 状态机的 confirm_bars、candidate_bars、cooldown_remaining_bars 等
//! 需要在多次 HTTP 请求间保持状态，否则确认期/冷却期无法在生产中生效。
//!
//! 当前实现：进程内存 HashMap，按 (source, symbol, interval, config_hash) 分片。
//! 后续可替换为 Redis / Postgres 持久化存储。

use std::collections::HashMap;
use std::sync::RwLock;

use crate::models::StateContext;

/// 存储键：(source, symbol, interval, config_hash)。
type StoreKey = (String, String, String, String);

/// 线程安全的内存 StateContext 存储。
pub struct StateContextStore {
    contexts: RwLock<HashMap<StoreKey, StateContext>>,
}

impl StateContextStore {
    /// 创建空的存储。
    pub fn new() -> Self {
        Self {
            contexts: RwLock::new(HashMap::new()),
        }
    }

    /// 获取或创建 StateContext。
    ///
    /// 如果 key 不存在，返回默认上下文（并存入 store）。
    pub fn get_or_create(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
        config_hash: &str,
    ) -> StateContext {
        let key = (
            source.to_ascii_lowercase(),
            symbol.to_ascii_uppercase(),
            interval.to_ascii_lowercase(),
            config_hash.to_string(),
        );

        // 先尝试读锁
        {
            let read = self.contexts.read().expect("StateContextStore read lock poisoned");
            if let Some(ctx) = read.get(&key) {
                return ctx.clone();
            }
        }

        // 需要写锁创建
        let mut write = self.contexts.write().expect("StateContextStore write lock poisoned");
        write
            .entry(key)
            .or_insert_with(StateContext::default)
            .clone()
    }

    /// 更新 StateContext（在 advance_state 之后调用）。
    pub fn save(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
        config_hash: &str,
        ctx: &StateContext,
    ) {
        let key = (
            source.to_ascii_lowercase(),
            symbol.to_ascii_uppercase(),
            interval.to_ascii_lowercase(),
            config_hash.to_string(),
        );
        let mut write = self.contexts.write().expect("StateContextStore write lock poisoned");
        write.insert(key, ctx.clone());
    }

    /// 移除一个上下文的存储（用于重置）。
    pub fn remove(
        &self,
        source: &str,
        symbol: &str,
        interval: &str,
        config_hash: &str,
    ) {
        let key = (
            source.to_ascii_lowercase(),
            symbol.to_ascii_uppercase(),
            interval.to_ascii_lowercase(),
            config_hash.to_string(),
        );
        let mut write = self.contexts.write().expect("StateContextStore write lock poisoned");
        write.remove(&key);
    }

    /// 返回当前存储的 key 数量（用于监控）。
    pub fn len(&self) -> usize {
        self.contexts.read().expect("StateContextStore read lock poisoned").len()
    }

    /// 清理超过指定时间未活跃的上下文。
    /// `max_age_ms`：最大允许的空闲时间。配合 `StateContext.last_transition_time` 使用。
    pub fn prune_stale(&self, max_age_ms: i64) -> usize {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut write = self.contexts.write().expect("StateContextStore write lock poisoned");
        let before = write.len();
        write.retain(|_, ctx| {
            ctx.last_transition_time
                .map(|t| now_ms - t < max_age_ms)
                .unwrap_or(true) // 没有 transition time 的保留
        });
        before - write.len()
    }
}

impl Default for StateContextStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_or_create_returns_same_context() {
        let store = StateContextStore::new();
        let hash = "sha256:test";

        let ctx1 = store.get_or_create("binance", "BTCUSDT", "5m", hash);
        assert_eq!(ctx1.candidate_bars, 0);
        assert!(ctx1.candidate_state.is_none());

        // 修改上下文
        let mut ctx2 = ctx1.clone();
        ctx2.candidate_bars = 2;
        ctx2.candidate_state = Some(crate::models::MarketState::RangeGrid);
        store.save("binance", "BTCUSDT", "5m", hash, &ctx2);

        // 重新获取应返回保存后的状态
        let ctx3 = store.get_or_create("binance", "BTCUSDT", "5m", hash);
        assert_eq!(ctx3.candidate_bars, 2);
        assert_eq!(ctx3.candidate_state, Some(crate::models::MarketState::RangeGrid));
    }

    #[test]
    fn test_key_isolation() {
        let store = StateContextStore::new();
        let hash = "sha256:test";

        let mut ctx_a = store.get_or_create("binance", "BTCUSDT", "5m", hash);
        ctx_a.candidate_bars = 3;
        store.save("binance", "BTCUSDT", "5m", hash, &ctx_a);

        let ctx_b = store.get_or_create("binance", "ETHUSDT", "5m", hash);
        assert_eq!(ctx_b.candidate_bars, 0); // 不同 symbol 独立

        let ctx_c = store.get_or_create("binance", "BTCUSDT", "1h", hash);
        assert_eq!(ctx_c.candidate_bars, 0); // 不同 interval 独立
    }

    #[test]
    fn test_config_hash_isolation() {
        let store = StateContextStore::new();

        let mut ctx1 = store.get_or_create("binance", "BTCUSDT", "5m", "sha256:v1");
        ctx1.candidate_bars = 5;
        store.save("binance", "BTCUSDT", "5m", "sha256:v1", &ctx1);

        let ctx2 = store.get_or_create("binance", "BTCUSDT", "5m", "sha256:v2");
        assert_eq!(ctx2.candidate_bars, 0); // 不同 config hash 独立
    }

    #[test]
    fn test_remove() {
        let store = StateContextStore::new();
        let hash = "sha256:test";

        let mut ctx = store.get_or_create("binance", "BTCUSDT", "5m", hash);
        ctx.candidate_bars = 3;
        store.save("binance", "BTCUSDT", "5m", hash, &ctx);

        store.remove("binance", "BTCUSDT", "5m", hash);
        let fresh = store.get_or_create("binance", "BTCUSDT", "5m", hash);
        assert_eq!(fresh.candidate_bars, 0);
    }
}
