/**
 * Compact human-readable duration formatting for analytics callouts and
 * histogram bin labels. Picks one unit — the largest that yields a value
 * ≥ 1 — and rounds. No locale or unit pluralization: callouts are tight
 * single-line strings ("5h", "1d", "12m").
 */

const MINUTE = 60;
const HOUR = 60 * 60;
const DAY = 24 * HOUR;

function roundOne(value: number): number {
  if (value >= 10) return Math.round(value);
  return Math.round(value * 10) / 10;
}

/** Format a non-negative duration in seconds as a compact string. */
export function formatDurationSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  if (seconds < MINUTE) return `${Math.round(seconds)}s`;
  if (seconds < HOUR) return `${roundOne(seconds / MINUTE)}m`;
  if (seconds < DAY) return `${roundOne(seconds / HOUR)}h`;
  return `${roundOne(seconds / DAY)}d`;
}

/**
 * Format a histogram bin's [start, end) range. `end` of `null` denotes the
 * open-ended last bin and produces e.g. "30d+".
 */
export function formatBinRange(startSeconds: number, endSeconds: number | null): string {
  if (endSeconds === null) return `${formatDurationSeconds(startSeconds)}+`;
  return `${formatDurationSeconds(startSeconds)}–${formatDurationSeconds(endSeconds)}`;
}

/**
 * Format a bucket-start ISO timestamp for chart X-axis labels. Falls back
 * to the raw input when the timestamp can't be parsed. Shared between the
 * over-time and in-flight charts so they stay visually consistent.
 */
export function formatBucketLabel(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
