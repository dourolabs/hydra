import type { TriggerSchedule } from "@hydra/api";

export type ScheduleKind = "cron" | "once";

export function scheduleKind(s: TriggerSchedule): ScheduleKind {
  return s.type === "cron" ? "cron" : "once";
}

/** Compact one-line preview, e.g. `cron: 0 9 * * 1-5` or `once @ 2026-06-10T09:00Z`. */
export function formatScheduleSummary(s: TriggerSchedule): string {
  if (s.type === "cron") {
    const tz = s.timezone ? ` (${s.timezone})` : "";
    return `cron: ${s.expression}${tz}`;
  }
  return `once @ ${s.at}`;
}
