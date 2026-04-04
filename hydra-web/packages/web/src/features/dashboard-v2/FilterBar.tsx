import { useState, useRef, useEffect } from "react";
import type { IssueStatus, PatchStatus, LabelRecord } from "@hydra/api";
import { useLabels } from "../labels/useLabels";
import styles from "./FilterBar.module.css";

const ISSUE_STATUSES: IssueStatus[] = [
  "open",
  "in-progress",
  "closed",
  "dropped",
  "rejected",
  "failed",
];

const PATCH_STATUSES: PatchStatus[] = ["Open", "Closed", "Merged", "ChangesRequested"];

const ISSUE_STATUS_COLORS: Record<IssueStatus, string> = {
  open: "var(--color-status-open)",
  "in-progress": "var(--color-status-in-progress)",
  closed: "var(--color-status-issue-closed)",
  failed: "var(--color-status-failed)",
  dropped: "var(--color-status-dropped)",
  rejected: "var(--color-status-rejected)",
};

const PATCH_STATUS_COLORS: Record<PatchStatus, string> = {
  Open: "var(--color-status-open)",
  Closed: "var(--color-status-issue-closed)",
  Merged: "var(--color-status-merged, var(--color-accent))",
  ChangesRequested: "var(--color-status-attention, var(--color-status-failed))",
};

type TabKind = "issues" | "patches" | "documents";

interface FilterBarProps {
  tabKind: TabKind;
  selectedIssueStatus: IssueStatus | null;
  onIssueStatusChange: (status: IssueStatus | null) => void;
  selectedPatchStatus: PatchStatus | null;
  onPatchStatusChange: (status: PatchStatus | null) => void;
  selectedLabelId: string | null;
  onLabelChange: (labelId: string | null) => void;
}

function StatusSelect<T extends string>({
  allStatuses,
  selected,
  onChange,
  label,
  colorMap,
}: {
  allStatuses: T[];
  selected: T | null;
  onChange: (next: T | null) => void;
  label: string;
  colorMap: Record<T, string>;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  const triggerLabel = selected ? `${label}: ${selected}` : label;

  return (
    <div className={styles.dropdownWrapper} ref={ref}>
      <button
        type="button"
        className={`${styles.dropdownTrigger} ${selected ? styles.dropdownTriggerActive : ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        {triggerLabel}
        <span className={styles.chevron}>&#x25BC;</span>
      </button>
      {open && (
        <div className={styles.dropdownMenu}>
          <div
            className={`${styles.labelItem} ${!selected ? styles.labelItemSelected : ""}`}
            onClick={() => {
              onChange(null);
              setOpen(false);
            }}
          >
            <span className={styles.noLabel}>All statuses</span>
          </div>
          {allStatuses.map((status) => (
            <div
              key={status}
              className={`${styles.labelItem} ${selected === status ? styles.labelItemSelected : ""}`}
              onClick={() => {
                onChange(status);
                setOpen(false);
              }}
            >
              <span className={styles.colorDot} style={{ backgroundColor: colorMap[status] }} />
              <span className={styles.labelName}>{status}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function LabelSelect({
  selectedLabelId,
  onChange,
}: {
  selectedLabelId: string | null;
  onChange: (labelId: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const { data: labels } = useLabels();

  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  const selectedLabel = labels?.find((l: LabelRecord) => l.label_id === selectedLabelId);
  const triggerLabel = selectedLabel ? `Label: ${selectedLabel.name}` : "Label";

  return (
    <div className={styles.dropdownWrapper} ref={ref}>
      <button
        type="button"
        className={`${styles.dropdownTrigger} ${selectedLabelId ? styles.dropdownTriggerActive : ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        {triggerLabel}
        <span className={styles.chevron}>&#x25BC;</span>
      </button>
      {open && (
        <div className={styles.dropdownMenu}>
          <div
            className={`${styles.labelItem} ${!selectedLabelId ? styles.labelItemSelected : ""}`}
            onClick={() => {
              onChange(null);
              setOpen(false);
            }}
          >
            <span className={styles.noLabel}>All labels</span>
          </div>
          {labels?.map((label: LabelRecord) => (
            <div
              key={label.label_id}
              className={`${styles.labelItem} ${selectedLabelId === label.label_id ? styles.labelItemSelected : ""}`}
              onClick={() => {
                onChange(label.label_id);
                setOpen(false);
              }}
            >
              <span className={styles.colorDot} style={{ backgroundColor: label.color }} />
              <span className={styles.labelName}>{label.name}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function FilterBar({
  tabKind,
  selectedIssueStatus,
  onIssueStatusChange,
  selectedPatchStatus,
  onPatchStatusChange,
  selectedLabelId,
  onLabelChange,
}: FilterBarProps) {
  if (tabKind === "documents") {
    return null;
  }

  return (
    <div className={styles.filterBar}>
      {tabKind === "issues" && (
        <>
          <StatusSelect
            allStatuses={ISSUE_STATUSES}
            selected={selectedIssueStatus}
            onChange={onIssueStatusChange}
            label="Status"
            colorMap={ISSUE_STATUS_COLORS}
          />
          <LabelSelect selectedLabelId={selectedLabelId} onChange={onLabelChange} />
        </>
      )}
      {tabKind === "patches" && (
        <StatusSelect
          allStatuses={PATCH_STATUSES}
          selected={selectedPatchStatus}
          onChange={onPatchStatusChange}
          label="Status"
          colorMap={PATCH_STATUS_COLORS}
        />
      )}
    </div>
  );
}
