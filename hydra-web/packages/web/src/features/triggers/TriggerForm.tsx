import { useCallback, useMemo } from "react";
import { Button, Input, Select, Textarea } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type { IssueStatus, IssueType, RepositoryRecord } from "@hydra/api";
import { useRepositories } from "../../hooks/useRepositories";
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

const ISSUE_STATUS_OPTIONS: SelectOption[] = [
  { value: "open", label: "open" },
  { value: "in-progress", label: "in-progress" },
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
  const repoOptions = useMemo<SelectOption[]>(
    () => buildRepoOptions(repos),
    [repos],
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
          <div key={idx} className={styles.actionCard}>
            <div className={styles.actionCardHead}>
              <span className={styles.actionCardLabel}>
                Action {idx + 1} · CreateIssue
              </span>
              {draft.actions.length > 1 && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => removeAction(idx)}
                >
                  Remove
                </Button>
              )}
            </div>

            <Select
              label="Type"
              options={ISSUE_TYPE_OPTIONS}
              value={a.type}
              onChange={(e) =>
                updateAction(idx, { type: e.target.value as IssueType })
              }
            />
            <Input
              label="Title template"
              placeholder="Daily standup {{scheduled_at}}"
              value={a.title}
              onChange={(e) => updateAction(idx, { title: e.target.value })}
              required
            />
            <Textarea
              label="Description template"
              placeholder="What did the team ship yesterday?"
              value={a.description}
              onChange={(e) =>
                updateAction(idx, { description: e.target.value })
              }
              rows={4}
              required
            />
            <Input
              label="Assignee (optional)"
              placeholder="agents/swe"
              value={a.assignee}
              onChange={(e) => updateAction(idx, { assignee: e.target.value })}
            />
            <Select
              label="Status (optional)"
              options={[{ value: "", label: "(default)" }, ...ISSUE_STATUS_OPTIONS]}
              value={a.status}
              onChange={(e) =>
                updateAction(idx, { status: e.target.value as IssueStatus | "" })
              }
            />
            <Select
              label="Repository (optional)"
              options={repoOptions}
              value={a.repoName}
              onChange={(e) => updateAction(idx, { repoName: e.target.value })}
            />
          </div>
        ))}
      </div>
    </>
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
