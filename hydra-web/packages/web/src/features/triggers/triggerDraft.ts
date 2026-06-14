import type {
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
  projectId: string;
  status: string;
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
    projectId: "",
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
  return actions.flatMap((a) => {
    if (a.type !== "create_issue") return [];
    return [
      {
        type: a.issue_type,
        title: a.title,
        description: a.description,
        assignee: a.assignee ?? "",
        projectId: a.project_id,
        status: a.status,
        repoName: a.session_settings?.repo_name ?? "",
      },
    ];
  });
}

function draftFromSchedule(s: TriggerSchedule): {
  kind: ScheduleKind;
  cronExpression: string;
  cronTimezone: string;
  onceAt: string;
} {
  if (s.type === "cron") {
    return {
      kind: "cron",
      cronExpression: s.expression,
      cronTimezone: s.timezone ?? "",
      onceAt: "",
    };
  }
  return {
    kind: "once",
    cronExpression: "",
    cronTimezone: "",
    onceAt: s.at,
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
    const tz = draft.cronTimezone.trim();
    return tz
      ? { type: "cron", expression: expr, timezone: tz }
      : { type: "cron", expression: expr };
  }
  const at = draft.onceAt.trim();
  if (!at) return null;
  return { type: "once", at };
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
    if (!a.projectId || !a.status) return null;
    const action: TriggerAction = {
      type: "create_issue",
      issue_type: a.type,
      title: a.title,
      description: a.description,
      project_id: a.projectId,
      status: a.status,
    };
    if (a.assignee.trim()) action.assignee = a.assignee.trim();
    if (a.repoName.trim()) {
      action.session_settings = { repo_name: a.repoName.trim() };
    }
    actions.push(action);
  }
  if (actions.length === 0) return null;
  return {
    enabled: draft.enabled,
    schedule,
    actions,
    creator,
  };
}
