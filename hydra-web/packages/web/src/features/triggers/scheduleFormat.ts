import type { TriggerSchedule } from "@hydra/api";

export type ScheduleKind = "cron" | "once";

export function scheduleKind(s: TriggerSchedule): ScheduleKind {
  return "Cron" in s ? "cron" : "once";
}

/** Compact one-line preview, e.g. `cron: 0 9 * * 1-5` or `once @ 2026-06-10T09:00Z`. */
export function formatScheduleSummary(s: TriggerSchedule): string {
  if ("Cron" in s) {
    const tz = s.Cron.timezone ? ` (${s.Cron.timezone})` : "";
    return `cron: ${s.Cron.expression}${tz}`;
  }
  return `once @ ${s.Once.at}`;
}
