/**
 * Analytics-specific label helpers (histogram bins, chart X-axis ticks).
 * Generic compact-duration formatting lives in `utils/time.ts`.
 */

import { formatDurationSeconds } from "../../utils/time";

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
