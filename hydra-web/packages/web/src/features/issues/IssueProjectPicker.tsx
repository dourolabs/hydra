import { useState } from "react";
import { Picker, PickerRow } from "@hydra/ui";
import { useProjects } from "../projects/useProjects";
import styles from "./IssueProjectPicker.module.css";

interface IssueProjectPickerProps {
  /** The issue's persisted project id; baseline for cancel detection. */
  projectId: string;
  /** Pending project id; `null` = no pending change. */
  pendingProjectId: string | null;
  /** Forwards the picked id; `null` when the user re-picks `projectId`. */
  onPendingChange: (projectId: string | null) => void;
  /** Hide the visual "Project" caption; the label still feeds `aria-label`. */
  hideLabel?: boolean;
}

export function IssueProjectPicker({
  projectId,
  pendingProjectId,
  onPendingChange,
  hideLabel,
}: IssueProjectPickerProps) {
  const [open, setOpen] = useState(false);
  const { data: projects } = useProjects();

  const activeProjectId = pendingProjectId ?? projectId;
  const selectedProject = projects?.find(
    (p) => p.project_id === activeProjectId,
  );

  const choose = (next: string) => {
    setOpen(false);
    if (next === activeProjectId) return;
    onPendingChange(next === projectId ? null : next);
  };

  return (
    <Picker
      label="Project"
      hideLabel={hideLabel}
      open={open}
      onToggle={() => setOpen((v) => !v)}
      wide
      data-testid="issue-project-picker"
      value={
        selectedProject ? (
          <span className={styles.pillContent}>
            <code className={styles.pillCode}>{selectedProject.project.key}</code>
          </span>
        ) : (
          <span className={styles.pillEmpty}>No project</span>
        )
      }
    >
      {(projects ?? []).map((p) => (
        <PickerRow
          key={p.project_id}
          active={activeProjectId === p.project_id}
          onClick={() => choose(p.project_id)}
          data-testid={`issue-project-option-${p.project.key}`}
        >
          <code className={styles.popCode}>{p.project.key}</code>
          <span className={styles.popSub}>{p.project.name}</span>
          <span className={styles.popSpacer} />
        </PickerRow>
      ))}
    </Picker>
  );
}
