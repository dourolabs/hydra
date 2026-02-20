/** Format a duration in milliseconds to a human-readable string. */
export function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
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
