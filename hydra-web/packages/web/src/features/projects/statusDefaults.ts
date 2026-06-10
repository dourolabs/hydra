import type { StatusDefinition } from "@hydra/api";
import { LABEL_COLOR_PALETTE } from "../../components/ColorPicker";

export function blankStatus(index: number): StatusDefinition {
  return {
    key: `status-${index + 1}`,
    label: "",
    color: LABEL_COLOR_PALETTE[index % LABEL_COLOR_PALETTE.length],
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    on_enter: null,
    prompt_path: null,
    auto_archive_after_seconds: null,
    position: (index + 1) * 100,
  };
}

export type AutoArchiveUnit = "hours" | "days" | "weeks";

export const AUTO_ARCHIVE_UNIT_SECONDS: Record<AutoArchiveUnit, number> = {
  hours: 3600,
  days: 86400,
  weeks: 7 * 86400,
};

/**
 * Pick the largest unit that divides `seconds` evenly so a round-number
 * server value (e.g. 1209600 = 14 days) doesn't render as 336 hours after a
 * round-trip. Falls back to `hours` when no whole-unit divisor matches.
 */
export function deriveAutoArchiveDisplay(seconds: number): {
  value: number;
  unit: AutoArchiveUnit;
} {
  if (seconds > 0 && seconds % AUTO_ARCHIVE_UNIT_SECONDS.weeks === 0) {
    return { value: seconds / AUTO_ARCHIVE_UNIT_SECONDS.weeks, unit: "weeks" };
  }
  if (seconds > 0 && seconds % AUTO_ARCHIVE_UNIT_SECONDS.days === 0) {
    return { value: seconds / AUTO_ARCHIVE_UNIT_SECONDS.days, unit: "days" };
  }
  return { value: seconds / AUTO_ARCHIVE_UNIT_SECONDS.hours, unit: "hours" };
}

/**
 * Read the wire-encoded duration as a JS number. ts-rs types i64 as
 * `bigint`, but the wire is a plain JSON number and JSON.parse hands back a
 * runtime `number` — this normalizes both so callers can do math on it.
 */
export function readAutoArchiveSeconds(s: StatusDefinition): number | null {
  const v = s.auto_archive_after_seconds;
  if (v == null) return null;
  return Number(v);
}

const STATUS_KEY_RE = /^[a-z0-9-]+$/;

export function validateStatusKey(
  key: string,
  existingKeys: ReadonlySet<string>,
): string | null {
  const trimmed = key.trim();
  if (!trimmed) return "Status key is required";
  if (!STATUS_KEY_RE.test(trimmed)) {
    return "Status key must be lowercase letters, digits, and dashes only";
  }
  if (existingKeys.has(trimmed)) {
    return `Status key '${trimmed}' already exists in this project`;
  }
  return null;
}

// Lowercase, dash-separated slug stripped of any character outside
// STATUS_KEY_RE. Collapsed and trimmed so callers get a key the wire layer
// already accepts.
export function slugifyStatusKey(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
