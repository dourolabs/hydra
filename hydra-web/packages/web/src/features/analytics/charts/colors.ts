/**
 * Shared chart palette. Values resolve through `tokens.css` so charts inherit
 * the project's warm-dark / oklch palette instead of a hard-coded "Flat UI"
 * one. recharts passes these through as inline SVG `fill`/`stroke` attributes
 * (and inline `style` for the tooltip + legend swatches), all of which resolve
 * `var()` references in modern browsers.
 *
 *  - `merged`  → emerald (matches `--s-closed`, the merged/closed status tone)
 *  - `created` → neutral grey
 *  - `closed`  → soft red (terminal-but-not-shipped)
 *  - `accent`  → emerald (project accent, used for single-series charts)
 *
 * Token-usage series ordered so the stack reads input → output → cache
 * (read then write). Hues span emerald → blue → violet → amber so adjacent
 * layers stay distinguishable against the dark surface.
 */
export const CHART_COLORS = {
  created: "var(--s-neutral)",
  merged: "var(--s-closed)",
  closed: "var(--s-failed)",
  accent: "var(--acc)",
  tokensInput: "var(--c-edit)",
  tokensOutput: "var(--s-open)",
  tokensCacheRead: "var(--c-violet)",
  tokensCacheWrite: "var(--s-blocked)",
} as const;

/** Shared recharts axis tick styling — kept here so a palette/theme tweak is a one-line change. */
export const AXIS_TICK = { fontSize: 11, fill: "var(--fg-2)" } as const;

/** Shared recharts tooltip `contentStyle` — matches the page's dark card surface. */
export const TOOLTIP_STYLE = {
  background: "var(--bg-1)",
  border: "1px solid var(--line)",
  color: "var(--fg-0)",
} as const;

/** Shared recharts CartesianGrid stroke color. */
export const GRID_STROKE = "var(--line)";
