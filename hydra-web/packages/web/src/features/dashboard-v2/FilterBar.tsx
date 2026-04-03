import { useState, useRef, useEffect, useCallback } from "react";
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

const PATCH_STATUSES: PatchStatus[] = [
  "Open",
  "Closed",
  "Merged",
  "ChangesRequested",
];

type TabKind = "issues" | "patches" | "documents";

interface FilterBarProps {
  tabKind: TabKind;
  selectedIssueStatuses: Set<IssueStatus>;
  onIssueStatusesChange: (statuses: Set<IssueStatus>) => void;
  selectedPatchStatuses: Set<PatchStatus>;
  onPatchStatusesChange: (statuses: Set<PatchStatus>) => void;
  selectedLabelId: string | null;
  onLabelChange: (labelId: string | null) => void;
}

function StatusMultiSelect<T extends string>({
  allStatuses,
  selected,
  onChange,
  label,
}: {
  allStatuses: T[];
  selected: Set<T>;
  onChange: (next: Set<T>) => void;
  label: string;
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

  const allSelected = selected.size === allStatuses.length;
  const noneSelected = selected.size === 0;
  const triggerLabel =
    allSelected || noneSelected
      ? label
      : selected.size === 1
        ? `${label}: ${[...selected][0]}`
        : `${label}: ${selected.size} selected`;

  const handleToggle = useCallback(
    (status: T) => {
      const next = new Set(selected);
      if (next.has(status)) {
        next.delete(status);
      } else {
        next.add(status);
      }
      onChange(next);
    },
    [selected, onChange],
  );

  const handleOnly = useCallback(
    (status: T, e: React.MouseEvent) => {
      e.stopPropagation();
      onChange(new Set([status]));
    },
    [onChange],
  );

  const isActive = !allSelected && !noneSelected;

  return (
    <div className={styles.dropdownWrapper} ref={ref}>
      <button
        type="button"
        className={`${styles.dropdownTrigger} ${isActive ? styles.dropdownTriggerActive : ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        {triggerLabel}
        <span className={styles.chevron}>&#x25BC;</span>
      </button>
      {open && (
        <div className={styles.dropdownMenu}>
          {allStatuses.map((status) => (
            <label
              key={status}
              className={styles.checkboxItem}
              onClick={() => handleToggle(status)}
            >
              <input
                type="checkbox"
                className={styles.checkbox}
                checked={selected.has(status)}
                onChange={() => {}}
              />
              <span className={styles.checkboxLabel}>{status}</span>
              <button
                type="button"
                className={styles.onlyButton}
                onClick={(e) => handleOnly(status, e)}
              >
                Only
              </button>
            </label>
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

  const selectedLabel = labels?.find(
    (l: LabelRecord) => l.label_id === selectedLabelId,
  );
  const triggerLabel = selectedLabel
    ? `Label: ${selectedLabel.name}`
    : "Label";

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
              <span
                className={styles.colorDot}
                style={{ backgroundColor: label.color }}
              />
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
  selectedIssueStatuses,
  onIssueStatusesChange,
  selectedPatchStatuses,
  onPatchStatusesChange,
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
          <StatusMultiSelect
            allStatuses={ISSUE_STATUSES}
            selected={selectedIssueStatuses}
            onChange={onIssueStatusesChange}
            label="Status"
          />
          <LabelSelect
            selectedLabelId={selectedLabelId}
            onChange={onLabelChange}
          />
        </>
      )}
      {tabKind === "patches" && (
        <StatusMultiSelect
          allStatuses={PATCH_STATUSES}
          selected={selectedPatchStatuses}
          onChange={onPatchStatusesChange}
          label="Status"
        />
      )}
    </div>
  );
}
