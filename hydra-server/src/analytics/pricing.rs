//! Token pricing constants and a `cost_usd` helper used by the
//! `/v1/analytics/token_usage/*` endpoints.
//!
//! Rates are baked in as Opus 4.8 only by design — this is the single
//! source of truth for token pricing, so updating a rate happens here.
//! See the parent issue's design note: "define the rate table in one
//! place so it's easy to update later."
//!
//! Rates checked on 2026-06-10 from the Anthropic pricing page:
//! <https://platform.claude.com/docs/en/docs/about-claude/pricing>
//!
//! Naming note: `TokenUsage.cache_creation_input_tokens` is what
//! Anthropic calls "cache writes" on the pricing table — we map that
//! field onto `cache_write_per_mtok` here. The 5-minute cache TTL rate
//! is the default; the 1-hour-TTL rate exists too but isn't surfaced on
//! the wire today.

use hydra_common::api::v1::sessions::TokenUsage;

/// Per-million-token rates for one Claude model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

/// Opus 4.8 published rates, 5-minute cache-write tier.
pub const OPUS_4_8_PRICING: TokenPricing = TokenPricing {
    input_per_mtok: 5.0,
    output_per_mtok: 25.0,
    cache_read_per_mtok: 0.50,
    cache_write_per_mtok: 6.25,
};

/// Blended dollar cost for a `TokenUsage` reading at Opus 4.8 rates.
pub fn cost_usd(usage: &TokenUsage) -> f64 {
    let pricing = OPUS_4_8_PRICING;
    let per_token = |count: u64, rate_per_mtok: f64| (count as f64) * rate_per_mtok / 1_000_000.0;
    per_token(usage.input_tokens, pricing.input_per_mtok)
        + per_token(usage.output_tokens, pricing.output_per_mtok)
        + per_token(usage.cache_read_input_tokens, pricing.cache_read_per_mtok)
        + per_token(
            usage.cache_creation_input_tokens,
            pricing.cache_write_per_mtok,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_usage_costs_zero() {
        let usage = TokenUsage::default();
        assert_eq!(cost_usd(&usage), 0.0);
    }

    /// Pinned fixture: 1M input + 500k output + 100k cache-read + 50k
    /// cache-write at Opus 4.8 rates. If any rate moves, this test
    /// catches it so the change is intentional.
    #[test]
    fn pinned_cost_for_known_mix_matches_opus_4_8_rates() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cache_read_input_tokens: 100_000,
            cache_creation_input_tokens: 50_000,
        };
        // 1M * 5.00 / 1M = 5.00
        // 500k * 25.00 / 1M = 12.50
        // 100k * 0.50 / 1M = 0.05
        // 50k * 6.25 / 1M = 0.3125
        // total = 17.8625
        let actual = cost_usd(&usage);
        assert!(
            (actual - 17.8625).abs() < 1e-9,
            "expected 17.8625, got {actual}"
        );
    }
}
