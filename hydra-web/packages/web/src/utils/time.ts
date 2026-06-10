const MINUTE = 60;
const HOUR = 60 * 60;
const DAY = 24 * HOUR;

function roundOne(value: number): number {
  if (value >= 10) return Math.round(value);
  return Math.round(value * 10) / 10;
}

/**
 * Compact single-unit duration formatter (`5s` / `12m` / `5h` / `1d`).
 * Picks the largest unit yielding a value ≥ 1 and rounds. Distinct from
 * `formatDuration(ms)` (two-unit, ms input) — used for analytics callouts
 * and histogram bin labels where space is tight.
 */
export function formatDurationSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  if (seconds < MINUTE) return `${Math.round(seconds)}s`;
  if (seconds < HOUR) return `${roundOne(seconds / MINUTE)}m`;
  if (seconds < DAY) return `${roundOne(seconds / HOUR)}h`;
  return `${roundOne(seconds / DAY)}d`;
}

/** Format a duration in milliseconds to a human-readable string. */
export function formatDuration(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) return `${minutes}m ${remainingSeconds}s`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}h ${remainingMinutes}m`;
}

/** Format an ISO timestamp to a locale string. */
export function formatTimestamp(ts: string): string {
  return new Date(ts).toLocaleString();
}

/** Format an ISO timestamp as a relative time string (e.g. "5m ago", "2h ago"). */
export function formatRelativeTime(ts: string): string {
  const ms = Date.now() - new Date(ts).getTime();
  if (ms < 0) return "just now";
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  return `${months}mo ago`;
}

/** Format an ISO timestamp as a compact relative value without an "ago" suffix
 *  (e.g. "5m", "2h", "1d", or "now"). The AgoTime component adds the suffix. */
export function shortRelativeTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "—";
  const sec = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day}d`;
  const mo = Math.floor(day / 30);
  return `${mo}mo`;
}

/** Compute runtime from start_time to end_time (or now). */
export function getRuntime(
  startTime: string | null | undefined,
  endTime: string | null | undefined,
): string {
  if (!startTime) return "\u2014";
  const start = new Date(startTime).getTime();
  const end = endTime ? new Date(endTime).getTime() : Date.now();
  return formatDuration(end - start);
}
