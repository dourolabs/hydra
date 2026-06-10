/**
 * Frontend mirror of `hydra-server/src/analytics/pricing.rs::OPUS_4_8_PRICING`.
 *
 * The Cost Over Time chart weights per-bucket token counts client-side
 * (the wire response intentionally only carries raw counts), so the rates
 * have to exist in two places. `pricing.rs` is the source of truth — if a
 * rate moves there, mirror the change here and update the pinned-fixture
 * test in `__tests__/CostCharts.test.tsx` so the drift is caught.
 */

export const INPUT_PER_MTOK = 5.0;
export const OUTPUT_PER_MTOK = 25.0;
export const CACHE_READ_PER_MTOK = 0.5;
export const CACHE_WRITE_PER_MTOK = 6.25;

export type CostKind =
  | "input_tokens"
  | "output_tokens"
  | "cache_read_input_tokens"
  | "cache_creation_input_tokens";

const RATE_BY_KIND: Record<CostKind, number> = {
  input_tokens: INPUT_PER_MTOK,
  output_tokens: OUTPUT_PER_MTOK,
  cache_read_input_tokens: CACHE_READ_PER_MTOK,
  cache_creation_input_tokens: CACHE_WRITE_PER_MTOK,
};

/** USD cost for `count` tokens of the given `kind` at Opus 4.8 rates. */
export function tokenCostUsd(kind: CostKind, count: number): number {
  return (count * RATE_BY_KIND[kind]) / 1_000_000;
}
