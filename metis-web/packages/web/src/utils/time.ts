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
