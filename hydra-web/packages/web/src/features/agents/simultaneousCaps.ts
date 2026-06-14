export const I32_MAX = 2147483647;

export const SIMULTANEOUS_PLACEHOLDER = "Unlimited";

export const SIMULTANEOUS_INTERACTIVE_HELP =
  "Limits the number of concurrent interactive sessions for this agent. Headless cap doesn't throttle interactive work and vice versa.";

export const SIMULTANEOUS_HEADLESS_HELP =
  "Limits the number of concurrent headless sessions for this agent. Headless cap doesn't throttle interactive work and vice versa.";

/** Render an agent's stored cap as an editable string. i32::MAX (the wire
 * default) renders as an empty input so the placeholder communicates the
 * "unlimited" intent rather than a literal 2147483647. */
export function formatSimultaneousCap(value: number): string {
  return value === I32_MAX ? "" : String(value);
}

/** Coerce the input value back to the wire shape. Empty / invalid / negative
 * inputs map to i32::MAX (unlimited) so the user can clear the cap by deleting
 * the digits. */
export function parseSimultaneousCap(raw: string): number {
  const trimmed = raw.trim();
  if (trimmed === "") return I32_MAX;
  const parsed = Number.parseInt(trimmed, 10);
  if (!Number.isFinite(parsed) || parsed < 0) return I32_MAX;
  return parsed;
}
