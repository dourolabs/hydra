import type {
  CreateIssueAction,
  IssueStatus,
  IssueType,
  TriggerAction,
  TriggerSchedule,
  UpsertTriggerRequest,
} from "@hydra/api";
import type { ScheduleKind } from "./scheduleFormat";

export interface ActionDraft {
  type: IssueType;
  title: string;
  description: string;
  assignee: string;
  status: IssueStatus | "";
  repoName: string;
}

export interface TriggerDraft {
  enabled: boolean;
  scheduleKind: ScheduleKind;
  cronExpression: string;
  cronTimezone: string;
  onceAt: string;
  actions: ActionDraft[];
}

export function emptyAction(): ActionDraft {
  return {
    type: "task",
    title: "",
    description: "",
    assignee: "",
    status: "",
    repoName: "",
  };
}

export function emptyTriggerDraft(): TriggerDraft {
  return {
    enabled: true,
    scheduleKind: "cron",
    cronExpression: "",
    cronTimezone: "",
    onceAt: "",
    actions: [emptyAction()],
  };
}

function draftFromTriggerActions(actions: TriggerAction[]): ActionDraft[] {
  if (actions.length === 0) return [emptyAction()];
  return actions.map((a) => {
    const ci = a.CreateIssue;
    return {
      type: ci.type,
      title: ci.title,
      description: ci.description,
      assignee: ci.assignee ?? "",
      status: ci.status ?? "",
      repoName: ci.session_settings?.repo_name ?? "",
    };
  });
}

function draftFromSchedule(s: TriggerSchedule): {
  kind: ScheduleKind;
  cronExpression: string;
  cronTimezone: string;
  onceAt: string;
} {
  if ("Cron" in s) {
    return {
      kind: "cron",
      cronExpression: s.Cron.expression,
      cronTimezone: s.Cron.timezone ?? "",
      onceAt: "",
    };
  }
  return {
    kind: "once",
    cronExpression: "",
    cronTimezone: "",
    onceAt: s.Once.at,
  };
}

export function initialDraftFromExisting(
  schedule: TriggerSchedule,
  enabled: boolean,
  actions: TriggerAction[],
): TriggerDraft {
  const sched = draftFromSchedule(schedule);
  return {
    enabled,
    scheduleKind: sched.kind,
    cronExpression: sched.cronExpression,
    cronTimezone: sched.cronTimezone,
    onceAt: sched.onceAt,
    actions: draftFromTriggerActions(actions),
  };
}

function buildSchedule(draft: TriggerDraft): TriggerSchedule | null {
  if (draft.scheduleKind === "cron") {
    const expr = draft.cronExpression.trim();
    if (!expr) return null;
    const cron: { expression: string; timezone?: string | null } = {
      expression: expr,
    };
    const tz = draft.cronTimezone.trim();
    if (tz) cron.timezone = tz;
    return { Cron: cron };
  }
  const at = draft.onceAt.trim();
  if (!at) return null;
  return { Once: { at } };
}

export function buildUpsertRequest(
  draft: TriggerDraft,
  creator: string,
): UpsertTriggerRequest | null {
  const schedule = buildSchedule(draft);
  if (!schedule) return null;
  const actions: TriggerAction[] = [];
  for (const a of draft.actions) {
    if (!a.title.trim() || !a.description.trim()) return null;
    const ci: CreateIssueAction = {
      type: a.type,
      title: a.title,
      description: a.description,
    };
    if (a.assignee.trim()) ci.assignee = a.assignee.trim();
    if (a.status) ci.status = a.status;
    if (a.repoName.trim()) {
      ci.session_settings = { repo_name: a.repoName.trim() };
    }
    actions.push({ CreateIssue: ci });
  }
  if (actions.length === 0) return null;
  return {
    enabled: draft.enabled,
    schedule,
    actions,
    creator,
  };
}
