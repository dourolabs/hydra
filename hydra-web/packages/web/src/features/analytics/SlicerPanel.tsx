import { useEffect, useMemo, useRef, useState } from "react";
import { Input, Panel, Select } from "@hydra/ui";
import { useProjects, useProjectStatuses } from "../projects/useProjects";
import { useRepositories } from "../../hooks/useRepositories";
import { ISSUE_TYPE_OPTIONS, type SlicerState } from "./slicerState";
import styles from "./SlicerPanel.module.css";

export interface SlicerPanelProps {
  state: SlicerState;
  onChange: (patch: Partial<SlicerState>) => void;
}

const FREE_TEXT_DEBOUNCE_MS = 300;

// Tracks a free-text slicer field locally so each keystroke doesn't rewrite
// the URL — and therefore every chart's React Query cache key. Re-syncs from
// `external` when the URL changes from outside (back/forward nav).
function useDebouncedTextSlicer(external: string | null, onCommit: (value: string | null) => void) {
  const [draft, setDraft] = useState(external ?? "");
  const lastCommittedRef = useRef(external ?? "");
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;

  useEffect(() => {
    const next = external ?? "";
    if (next !== lastCommittedRef.current) {
      lastCommittedRef.current = next;
      setDraft(next);
    }
  }, [external]);

  useEffect(() => {
    if (draft === lastCommittedRef.current) return;
    const timeout = setTimeout(() => {
      lastCommittedRef.current = draft;
      onCommitRef.current(draft || null);
    }, FREE_TEXT_DEBOUNCE_MS);
    return () => clearTimeout(timeout);
  }, [draft]);

  return [draft, setDraft] as const;
}

export function SlicerPanel({ state, onChange }: SlicerPanelProps) {
  const { data: projects } = useProjects();
  const { data: statusesResp } = useProjectStatuses(state.projectId);
  const { data: repos } = useRepositories();

  const [assigneeDraft, setAssigneeDraft] = useDebouncedTextSlicer(state.assignee, (assignee) =>
    onChange({ assignee }),
  );
  const [creatorDraft, setCreatorDraft] = useDebouncedTextSlicer(state.creator, (creator) =>
    onChange({ creator }),
  );

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
              value={assigneeDraft}
              onChange={(e) => setAssigneeDraft(e.target.value)}
              data-testid="slicer-assignee"
            />
          </div>

          <div className={styles.field}>
            <label className={styles.label} htmlFor="slicer-creator">
              Creator
            </label>
            <Input
              id="slicer-creator"
              type="text"
              placeholder="username"
              value={creatorDraft}
              onChange={(e) => setCreatorDraft(e.target.value)}
              data-testid="slicer-creator"
            />
          </div>
        </div>
      </Panel>
    </aside>
  );
}
