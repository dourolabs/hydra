/**
 * Shared chart palette. Picked from the project's status / brand colors so
 * the analytics page reads in the same idiom as IssuesListPage badges:
 *  - `merged`  → green   (matches the merged/closed status tone)
 *  - `created` → neutral grey
 *  - `closed`  → soft red (terminal-but-not-shipped)
 *  - `accent`  → blue (single-series default)
 */
export const CHART_COLORS = {
  created: "#7c8694",
  merged: "#2ecc71",
  closed: "#e57373",
  accent: "#3498db",
} as const;

/** Shared recharts axis tick styling — kept here so a palette/theme tweak is a one-line change. */
export const AXIS_TICK = { fontSize: 11, fill: "#888" } as const;

/** Shared recharts tooltip `contentStyle` — matches the page's dark card surface. */
export const TOOLTIP_STYLE = {
  background: "#0e0e0e",
  border: "1px solid #2a2a2a",
} as const;

/** Shared recharts CartesianGrid stroke color. */
export const GRID_STROKE = "#2a2a2a";
