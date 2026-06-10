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
