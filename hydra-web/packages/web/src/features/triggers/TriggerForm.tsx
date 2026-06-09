import { useCallback, useEffect, useMemo } from "react";
import { Button, Input, Select, Textarea } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type { IssueType, RepositoryRecord } from "@hydra/api";
import { useRepositories } from "../../hooks/useRepositories";
import {
  useProjects,
  useProjectStatuses,
} from "../projects/useProjects";
import type { ScheduleKind } from "./scheduleFormat";
import type { ActionDraft, TriggerDraft } from "./triggerDraft";
import { emptyAction } from "./triggerDraft";
import styles from "./TriggersSection.module.css";

const ISSUE_TYPE_OPTIONS: SelectOption[] = [
  { value: "task", label: "task" },
  { value: "bug", label: "bug" },
  { value: "feature", label: "feature" },
  { value: "chore", label: "chore" },
  { value: "merge-request", label: "merge-request" },
  { value: "review-request", label: "review-request" },
];

const SCHEDULE_KIND_OPTIONS: SelectOption[] = [
  { value: "cron", label: "Cron" },
  { value: "once", label: "Once" },
];

interface TriggerFormProps {
  draft: TriggerDraft;
  onChange: (draft: TriggerDraft) => void;
}

export function TriggerForm({ draft, onChange }: TriggerFormProps) {
  const { data: repos } = useRepositories();
  const { data: projects } = useProjects();
  const repoOptions = useMemo<SelectOption[]>(
    () => buildRepoOptions(repos),
    [repos],
  );
  const projectOptions = useMemo<SelectOption[]>(
    () => buildProjectOptions(projects),
    [projects],
  );

  const setActions = useCallback(
    (actions: ActionDraft[]) => onChange({ ...draft, actions }),
    [draft, onChange],
  );

  const updateAction = useCallback(
    (idx: number, patch: Partial<ActionDraft>) => {
      const next = draft.actions.map((a, i) =>
        i === idx ? { ...a, ...patch } : a,
      );
      setActions(next);
    },
    [draft.actions, setActions],
  );

  const addAction = useCallback(
    () => setActions([...draft.actions, emptyAction()]),
    [draft.actions, setActions],
  );

  const removeAction = useCallback(
    (idx: number) => {
      if (draft.actions.length <= 1) return;
      setActions(draft.actions.filter((_, i) => i !== idx));
    },
    [draft.actions, setActions],
  );

  return (
    <>
      <label className={styles.checkboxLabel}>
        <input
          type="checkbox"
          checked={draft.enabled}
          onChange={(e) => onChange({ ...draft, enabled: e.target.checked })}
        />
        Enabled
      </label>

      <Select
        label="Schedule"
        options={SCHEDULE_KIND_OPTIONS}
        value={draft.scheduleKind}
        onChange={(e) =>
          onChange({
            ...draft,
            scheduleKind: e.target.value as ScheduleKind,
          })
        }
      />

      {draft.scheduleKind === "cron" ? (
        <>
          <Input
            label="Cron expression"
            placeholder="0 9 * * 1-5"
            value={draft.cronExpression}
            onChange={(e) =>
              onChange({ ...draft, cronExpression: e.target.value })
            }
            required
          />
          <Input
            label="Timezone (optional)"
            placeholder="UTC"
            value={draft.cronTimezone}
            onChange={(e) =>
              onChange({ ...draft, cronTimezone: e.target.value })
            }
          />
        </>
      ) : (
        <Input
          label="Fire at (RFC 3339)"
          placeholder="2026-06-10T09:00:00Z"
          value={draft.onceAt}
          onChange={(e) => onChange({ ...draft, onceAt: e.target.value })}
          required
        />
      )}

      <div className={styles.actionsList}>
        <div className={styles.actionsListHead}>
          <span className={styles.actionsListTitle}>Actions</span>
          <Button variant="secondary" size="sm" onClick={addAction}>
            Add action
          </Button>
        </div>

        {draft.actions.map((a, idx) => (
          <ActionCard
            key={idx}
            idx={idx}
            action={a}
            projectOptions={projectOptions}
            repoOptions={repoOptions}
            canRemove={draft.actions.length > 1}
            onUpdate={(patch) => updateAction(idx, patch)}
            onRemove={() => removeAction(idx)}
          />
        ))}
      </div>
    </>
  );
}

interface ActionCardProps {
  idx: number;
  action: ActionDraft;
  projectOptions: SelectOption[];
  repoOptions: SelectOption[];
  canRemove: boolean;
  onUpdate: (patch: Partial<ActionDraft>) => void;
  onRemove: () => void;
}

function ActionCard({
  idx,
  action,
  projectOptions,
  repoOptions,
  canRemove,
  onUpdate,
  onRemove,
}: ActionCardProps) {
  // Status options follow the action's selected project. The hook falls
  // back to the default project when given a falsy id; that fetch is
  // harmless because the picker stays disabled until the user picks a
  // project, so the loaded options are never shown.
  const { data: projectStatuses } = useProjectStatuses(
    action.projectId || null,
  );
  const statusEntries = useMemo(
    () =>
      action.projectId ? (projectStatuses?.statuses ?? []) : [],
    [action.projectId, projectStatuses],
  );
  const statusOptions = useMemo<SelectOption[]>(
    () =>
      statusEntries.map((s) => ({
        value: s.key,
        label: s.label || s.key,
      })),
    [statusEntries],
  );

  // If the project's status list loads and the current draft `status` is
  // not one of the project's keys, clear it so the user is forced to pick
  // a valid one. Empty stays empty.
  useEffect(() => {
    if (!action.status) return;
    if (statusEntries.length === 0) return;
    if (!statusEntries.some((s) => s.key === action.status)) {
      onUpdate({ status: "" });
    }
  }, [action.status, statusEntries, onUpdate]);

  return (
    <div className={styles.actionCard}>
      <div className={styles.actionCardHead}>
        <span className={styles.actionCardLabel}>
          Action {idx + 1} · CreateIssue
        </span>
        {canRemove && (
          <Button variant="ghost" size="sm" onClick={onRemove}>
            Remove
          </Button>
        )}
      </div>

      <Select
        label="Type"
        options={ISSUE_TYPE_OPTIONS}
        value={action.type}
        onChange={(e) => onUpdate({ type: e.target.value as IssueType })}
      />
      <Input
        label="Title template"
        placeholder="Daily standup {{scheduled_at}}"
        value={action.title}
        onChange={(e) => onUpdate({ title: e.target.value })}
        required
      />
      <Textarea
        label="Description template"
        placeholder="What did the team ship yesterday?"
        value={action.description}
        onChange={(e) => onUpdate({ description: e.target.value })}
        rows={4}
        required
      />
      <Input
        label="Assignee (optional)"
        placeholder="agents/swe"
        value={action.assignee}
        onChange={(e) => onUpdate({ assignee: e.target.value })}
      />
      <Select
        label="Project"
        options={[
          { value: "", label: "Select a project…" },
          ...projectOptions,
        ]}
        value={action.projectId}
        onChange={(e) =>
          // Reset status so the next render re-derives it against the new
          // project's status list.
          onUpdate({ projectId: e.target.value, status: "" })
        }
        required
      />
      <Select
        label="Status"
        options={[
          {
            value: "",
            label: action.projectId
              ? "Select a status…"
              : "Pick a project first",
          },
          ...statusOptions,
        ]}
        value={action.status}
        onChange={(e) => onUpdate({ status: e.target.value })}
        disabled={!action.projectId}
        required
      />
      <Select
        label="Repository (optional)"
        options={repoOptions}
        value={action.repoName}
        onChange={(e) => onUpdate({ repoName: e.target.value })}
      />
    </div>
  );
}

function buildRepoOptions(
  repos: RepositoryRecord[] | undefined,
): SelectOption[] {
  const options: SelectOption[] = [{ value: "", label: "None" }];
  if (repos) {
    for (const r of repos) {
      options.push({ value: r.name, label: r.name });
    }
  }
  return options;
}

function buildProjectOptions(
  projects:
    | { project_id: string; project: { key: string; name: string } }[]
    | undefined,
): SelectOption[] {
  if (!projects) return [];
  return projects.map((p) => ({
    value: p.project_id,
    label: `${p.project.key} — ${p.project.name}`,
  }));
}
