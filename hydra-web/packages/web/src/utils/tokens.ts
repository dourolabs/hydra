/**
 * Format a token count as a short human-readable string.
 *
 * Examples: 0 → "0", 850 → "850", 4000 → "4k", 4900 → "4.9k",
 * 42000 → "42k", 1_500_000 → "1.5M".
 */
export function formatTokenCount(n: number | bigint | null | undefined): string {
  if (n === null || n === undefined) return "0";
  const num = typeof n === "bigint" ? Number(n) : n;
  if (!Number.isFinite(num) || num < 0) return "0";
  if (num < 1000) return String(Math.round(num));
  if (num < 1_000_000) {
    const k = num / 1000;
    return k < 10 ? `${trimDecimal(k, 1)}k` : `${Math.round(k)}k`;
  }
  const m = num / 1_000_000;
  return m < 10 ? `${trimDecimal(m, 1)}M` : `${Math.round(m)}M`;
}

function trimDecimal(n: number, places: number): string {
  const fixed = n.toFixed(places);
  return fixed.replace(/\.0+$/, "");
}
