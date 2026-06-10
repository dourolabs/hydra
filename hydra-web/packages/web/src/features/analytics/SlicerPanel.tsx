import { useMemo } from "react";
import { Input, Panel, Select } from "@hydra/ui";
import { useProjects, useProjectStatuses } from "../projects/useProjects";
import { useRepositories } from "../../hooks/useRepositories";
import { ISSUE_TYPE_OPTIONS, type SlicerState } from "./slicerState";
import styles from "./SlicerPanel.module.css";

export interface SlicerPanelProps {
  state: SlicerState;
  onChange: (patch: Partial<SlicerState>) => void;
}

export function SlicerPanel({ state, onChange }: SlicerPanelProps) {
  const { data: projects } = useProjects();
  const { data: statusesResp } = useProjectStatuses(state.projectId);
  const { data: repos } = useRepositories();

  const projectOptions = useMemo(
    () => [
      { value: "", label: "All projects" },
      ...(projects ?? []).map((p) => ({
        value: p.project_id,
        label: p.project.name || p.project.key,
      })),
    ],
    [projects],
  );

  const statusOptions = useMemo(() => statusesResp?.statuses ?? [], [statusesResp]);

  const repoOptions = useMemo(
    () => [
      { value: "", label: "All repos" },
      ...(repos ?? []).map((r) => ({ value: r.name, label: r.name })),
    ],
    [repos],
  );

  const handleStatusToggle = (key: string, checked: boolean) => {
    const next = checked ? [...state.statusKeys, key] : state.statusKeys.filter((s) => s !== key);
    onChange({ statusKeys: next });
  };

  const handleIssueTypeToggle = (type: (typeof ISSUE_TYPE_OPTIONS)[number], checked: boolean) => {
    const next = checked ? [...state.issueTypes, type] : state.issueTypes.filter((t) => t !== type);
    onChange({ issueTypes: next });
  };

  return (
    <aside className={styles.panel} aria-label="Slicers" data-testid="slicer-panel">
      <Panel header="Slicers">
        <div className={styles.fields}>
          <div className={styles.field}>
            <Select
              id="slicer-project"
              label="Project"
              options={projectOptions}
              value={state.projectId ?? ""}
              onChange={(e) =>
                onChange({
                  projectId: e.target.value || null,
                  statusKeys: [],
                })
              }
              data-testid="slicer-project"
            />
          </div>

          <div className={styles.field}>
            <label className={styles.label}>
              Status
              {!state.projectId && <span className={styles.hint}> · select a project</span>}
            </label>
            <div
              className={styles.checklist}
              aria-disabled={!state.projectId}
              data-testid="slicer-status"
            >
              {!state.projectId && <div className={styles.empty}>No project selected</div>}
              {state.projectId &&
                statusOptions.map((s) => {
                  const checked = state.statusKeys.includes(s.key);
                  return (
                    <label key={s.key} className={styles.checkRow}>
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(e) => handleStatusToggle(s.key, e.target.checked)}
                        data-testid={`slicer-status-${s.key}`}
                      />
                      <span>{s.label}</span>
                    </label>
                  );
                })}
            </div>
          </div>

          <div className={styles.field}>
            <Select
              id="slicer-repo"
              label="Repo"
              options={repoOptions}
              value={state.repoName ?? ""}
              onChange={(e) => onChange({ repoName: e.target.value || null })}
              data-testid="slicer-repo"
            />
          </div>

          <div className={styles.field}>
            <label className={styles.label}>
              Issue type
              <span className={styles.hint}> · issues charts only</span>
            </label>
            <div className={styles.checklist} data-testid="slicer-issue-type">
              {ISSUE_TYPE_OPTIONS.map((t) => {
                const checked = state.issueTypes.includes(t);
                return (
                  <label key={t} className={styles.checkRow}>
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={(e) => handleIssueTypeToggle(t, e.target.checked)}
                      data-testid={`slicer-issue-type-${t}`}
                    />
                    <span>{t}</span>
                  </label>
                );
              })}
            </div>
          </div>

          <div className={styles.field}>
            <label className={styles.label} htmlFor="slicer-assignee">
              Assignee
              <span className={styles.hint}> · issues charts only</span>
            </label>
            <Input
              id="slicer-assignee"
              type="text"
              placeholder="users/alice or agents/bot"
              value={state.assignee ?? ""}
              onChange={(e) => onChange({ assignee: e.target.value || null })}
              data-testid="slicer-assignee"
            />
          </div>

          <div className={styles.field}>
            <Input
              id="slicer-creator"
              label="Creator"
              type="text"
              placeholder="username"
              value={state.creator ?? ""}
              onChange={(e) => onChange({ creator: e.target.value || null })}
              data-testid="slicer-creator"
            />
          </div>
        </div>
      </Panel>
    </aside>
  );
}
