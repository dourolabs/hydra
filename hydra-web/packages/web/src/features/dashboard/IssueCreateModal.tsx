import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { Avatar, Button, Icons, Kbd, Picker, PickerRow, TypeChip } from "@hydra/ui";
import type { IssueType, LabelRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useFormDraft } from "../../hooks/useFormDraft";
import { useFormModal } from "../../hooks/useFormModal";
import { useAuth } from "../auth/useAuth";
import { actorDisplayName } from "../../api/auth";
import { useLabels } from "../labels/useLabels";
import { LABEL_COLOR_PALETTE } from "../labels/LabelPicker";
import styles from "./IssueCreateModal.module.css";

type PickerKey = "type" | "assignee" | "repo" | "labels" | null;

const ISSUE_TYPES: IssueType[] = ["task", "bug", "feature", "chore"];
const LABEL_PILL_MAX_INLINE = 2;

interface IssueCreateModalProps {
  open: boolean;
  onClose: () => void;
  assignees: string[];
}

export function IssueCreateModal({ open, onClose, assignees }: IssueCreateModalProps) {
  const { user } = useAuth();
  const { data: repos } = useRepositories();
  const { data: labels } = useLabels();
  const currentUsername = user ? actorDisplayName(user.actor) : "";

  const [title, setTitle, clearTitleDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:title",
    "",
  );
  const [description, setDescription, clearDescriptionDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:description",
    "",
  );
  const [issueType, setIssueType, clearIssueTypeDraft] = useFormDraft<IssueType>(
    "hydra:draft:issue-create-modal:issueType",
    "task",
  );
  const [assignee, setAssignee, clearAssigneeDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:assignee",
    "",
  );
  const [repoName, setRepoName, clearRepoNameDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:repoName",
    "",
  );
  const [labelNames, setLabelNames, clearLabelNamesDraft] = useFormDraft<string[]>(
    "hydra:draft:issue-create-modal:labelNames",
    [],
  );

  const [picker, setPicker] = useState<PickerKey>(null);
  const titleInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!open) {
      setPicker(null);
      return;
    }
    const t = window.setTimeout(() => titleInputRef.current?.focus(), 0);
    return () => window.clearTimeout(t);
  }, [open]);

  // Esc closes the modal globally.
  useEffect(() => {
    if (!open) return;
    const handler = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (picker) {
          setPicker(null);
        } else {
          onClose();
        }
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose, picker]);

  const resetForm = useCallback(() => {
    setTitle("");
    setDescription("");
    setIssueType("task");
    setAssignee("");
    setRepoName("");
    setLabelNames([]);
    clearTitleDraft();
    clearDescriptionDraft();
    clearIssueTypeDraft();
    clearAssigneeDraft();
    clearRepoNameDraft();
    clearLabelNamesDraft();
  }, [
    setTitle,
    setDescription,
    setIssueType,
    setAssignee,
    setRepoName,
    setLabelNames,
    clearTitleDraft,
    clearDescriptionDraft,
    clearIssueTypeDraft,
    clearAssigneeDraft,
    clearRepoNameDraft,
    clearLabelNamesDraft,
  ]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    {
      title: string;
      description: string;
      creator: string;
      type: IssueType;
      assignee?: string;
      repoName?: string;
      labelNames?: string[];
    },
    { issue_id: string }
  >({
    mutationFn: (params) =>
      apiClient.createIssue({
        issue: {
          type: params.type,
          title: params.title,
          description: params.description,
          creator: params.creator,
          progress: "",
          status: "open",
          dependencies: [],
          patches: [],
          // Phase 4b: assignee is a typed `Principal`. The picker
          // surfaces agent names today, so wire as `Principal::Agent`.
          ...(params.assignee && {
            assignee: { kind: "agent", name: params.assignee } as const,
          }),
          ...(params.repoName && {
            session_settings: { repo_name: params.repoName },
          }),
        },
        session_id: null,
        ...(params.labelNames &&
          params.labelNames.length > 0 && {
            label_names: params.labelNames,
          }),
      }),
    invalidateKeys: [["issues"]],
    successMessage: (data) => `Issue ${data.issue_id} created`,
    onSuccess: () => {
      resetForm();
      onClose();
    },
  });

  const handleSubmit = useCallback(() => {
    const desc = description.trim();
    if (!desc) return;
    mutation.mutate({
      title: title.trim(),
      description: desc,
      creator: currentUsername,
      type: issueType,
      ...(assignee && { assignee }),
      ...(repoName && { repoName }),
      ...(labelNames.length > 0 && { labelNames }),
    });
  }, [
    title,
    description,
    currentUsername,
    issueType,
    assignee,
    repoName,
    labelNames,
    mutation,
  ]);

  // X close / backdrop / Esc — preserves drafts.
  const requestClose = useCallback(() => {
    handleClose(onClose);
  }, [handleClose, onClose]);

  // Cancel button — clears drafts before closing.
  const handleCancel = useCallback(() => {
    handleClose(onClose, resetForm);
  }, [handleClose, onClose, resetForm]);

  const onSubmitKeyDown = (e: KeyboardEvent<HTMLDivElement>) =>
    handleKeyDown(e, handleSubmit);

  const isMac = typeof navigator !== "undefined" && navigator.platform.includes("Mac");

  const labelColorByName = useMemo(() => {
    const map = new Map<string, string>();
    for (const l of labels ?? []) map.set(l.name, l.color);
    return map;
  }, [labels]);

  const toggleLabel = useCallback(
    (name: string) => {
      if (labelNames.includes(name)) {
        setLabelNames(labelNames.filter((n) => n !== name));
      } else {
        setLabelNames([...labelNames, name]);
      }
    },
    [labelNames, setLabelNames],
  );

  if (!open) return null;

  const repoEntries = repos ?? [];
  const canSubmit = description.trim().length > 0 && !isPending;

  return (
    <div
      className={styles.backdrop}
      onClick={(e) => {
        if (e.target === e.currentTarget) requestClose();
      }}
      data-testid="issue-create-backdrop"
    >
      <div
        className={styles.modal}
        role="dialog"
        aria-modal="true"
        aria-label="Create issue"
        data-testid="issue-create-modal"
        onKeyDown={onSubmitKeyDown}
      >
        <div className={styles.head}>
          <div className={styles.headLeft}>
            <span className={styles.headIcon}>
              <Icons.IconIssue size={16} />
            </span>
            <span className={styles.headTitle}>New issue</span>
          </div>
          <button
            type="button"
            className={styles.close}
            onClick={requestClose}
            aria-label="Close"
          >
            <Icons.IconX size={14} />
          </button>
        </div>

        <div className={styles.body}>
          <input
            ref={titleInputRef}
            className={styles.title}
            placeholder="Issue title…"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
          <textarea
            className={styles.desc}
            placeholder="Describe the issue. Acceptance criteria, repro steps, anything an agent needs to know…"
            rows={6}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />

          <div className={styles.pickers}>
            <Picker
              label="Type"
              open={picker === "type"}
              onToggle={() => setPicker(picker === "type" ? null : "type")}
              value={<TypeChip type={issueType} />}
            >
              {ISSUE_TYPES.map((t) => (
                <PickerRow
                  key={t}
                  active={issueType === t}
                  onClick={() => {
                    setIssueType(t);
                    setPicker(null);
                  }}
                >
                  <TypeChip type={t} />
                  <span className={styles.popSpacer} />
                </PickerRow>
              ))}
            </Picker>

            <Picker
              label="Assignee"
              open={picker === "assignee"}
              onToggle={() => setPicker(picker === "assignee" ? null : "assignee")}
              wide
              value={
                assignee ? (
                  <span className={styles.pillContent}>
                    <Avatar name={assignee} kind="agent" size="md" />
                    <span>{assignee}</span>
                  </span>
                ) : (
                  <span className={styles.pillEmpty}>Unassigned</span>
                )
              }
            >
              <PickerRow
                active={!assignee}
                onClick={() => {
                  setAssignee("");
                  setPicker(null);
                }}
              >
                <span className={styles.pillEmpty}>Unassigned</span>
                <span className={styles.popSpacer} />
              </PickerRow>
              {assignees.length > 0 && (
                <>
                  <div className={styles.popSection}>Agents</div>
                  {assignees.map((name) => (
                    <PickerRow
                      key={name}
                      active={assignee === name}
                      onClick={() => {
                        setAssignee(name);
                        setPicker(null);
                      }}
                    >
                      <Avatar name={name} kind="agent" size="md" />
                      <span>{name}</span>
                      <span className={styles.popSpacer} />
                    </PickerRow>
                  ))}
                </>
              )}
            </Picker>

            <Picker
              label="Repository"
              open={picker === "repo"}
              onToggle={() => setPicker(picker === "repo" ? null : "repo")}
              wide
              value={
                repoName ? (
                  <code className={styles.pillCode}>{repoName}</code>
                ) : (
                  <span className={styles.pillEmpty}>None</span>
                )
              }
            >
              <PickerRow
                active={!repoName}
                onClick={() => {
                  setRepoName("");
                  setPicker(null);
                }}
              >
                <span className={styles.pillEmpty}>None</span>
                <span className={styles.popSpacer} />
              </PickerRow>
              {repoEntries.map((r) => (
                <PickerRow
                  key={r.name}
                  active={repoName === r.name}
                  onClick={() => {
                    setRepoName(r.name);
                    setPicker(null);
                  }}
                >
                  <Icons.IconRepo size={14} />
                  <code className={styles.popCode}>{r.name}</code>
                  <span className={styles.popSpacer} />
                </PickerRow>
              ))}
            </Picker>

            <LabelsPicker
              open={picker === "labels"}
              onToggle={() => setPicker(picker === "labels" ? null : "labels")}
              selectedNames={labelNames}
              onToggleLabel={toggleLabel}
              labels={labels ?? []}
              labelColorByName={labelColorByName}
            />
          </div>
        </div>

        <div className={styles.foot}>
          <span className={styles.footSpacer} />
          <span className={styles.footHint}>
            <Kbd>{isMac ? "⌘" : "Ctrl"}</Kbd>
            <Kbd>↵</Kbd> submit
          </span>
          <Button variant="ghost" size="sm" onClick={handleCancel}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={handleSubmit}
            disabled={!canSubmit}
          >
            <Icons.IconPlus size={14} />
            {isPending ? "Creating…" : "Create issue"}
          </Button>
        </div>
      </div>
    </div>
  );
}

/** ── Labels Picker ── */

interface LabelsPickerProps {
  open: boolean;
  onToggle: () => void;
  selectedNames: string[];
  onToggleLabel: (name: string) => void;
  labels: LabelRecord[];
  labelColorByName: Map<string, string>;
}

function LabelsPicker({
  open,
  onToggle,
  selectedNames,
  onToggleLabel,
  labels,
  labelColorByName,
}: LabelsPickerProps) {
  const [search, setSearch] = useState("");
  const [newLabelColor, setNewLabelColor] = useState(LABEL_COLOR_PALETTE[0]);
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  // Reset search and focus input when opening; reset color choice when closing.
  useEffect(() => {
    if (open) {
      const t = window.setTimeout(() => searchInputRef.current?.focus(), 0);
      return () => window.clearTimeout(t);
    }
    setSearch("");
    setNewLabelColor(LABEL_COLOR_PALETTE[0]);
  }, [open]);

  const trimmed = search.trim();
  const filteredLabels = labels.filter((l) =>
    l.name.toLowerCase().includes(trimmed.toLowerCase()),
  );
  const showCreateOption =
    trimmed.length > 0 &&
    !labels.some((l) => l.name.toLowerCase() === trimmed.toLowerCase()) &&
    !selectedNames.includes(trimmed);

  const handleCreate = () => {
    if (!trimmed) return;
    onToggleLabel(trimmed);
    setSearch("");
    setNewLabelColor(LABEL_COLOR_PALETTE[0]);
  };

  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      if (showCreateOption) {
        handleCreate();
        return;
      }
      const exact = labels.find(
        (l) => l.name.toLowerCase() === trimmed.toLowerCase(),
      );
      if (exact) {
        onToggleLabel(exact.name);
        setSearch("");
      }
    }
  };

  const inlineNames = selectedNames.slice(0, LABEL_PILL_MAX_INLINE);
  const overflowCount = Math.max(0, selectedNames.length - LABEL_PILL_MAX_INLINE);

  const colorFor = (name: string) =>
    labelColorByName.get(name) ?? newLabelColor;

  return (
    <Picker
      label="Labels"
      open={open}
      onToggle={onToggle}
      wide
      value={
        selectedNames.length === 0 ? (
          <span className={styles.pillEmpty}>No labels</span>
        ) : (
          <span className={styles.pillContent}>
            {inlineNames.map((name) => (
              <span key={name} className={styles.pillLabelChip}>
                <span
                  className={styles.pillLabelDot}
                  style={{ backgroundColor: colorFor(name) }}
                />
                <span className={styles.pillLabelName}>{name}</span>
              </span>
            ))}
            {overflowCount > 0 && (
              <span className={styles.pillLabelMore}>+{overflowCount}</span>
            )}
          </span>
        )
      }
    >
      <div className={styles.popSearchWrap}>
        <input
          ref={searchInputRef}
          className={styles.popSearch}
          type="text"
          placeholder="Search or create…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={onSearchKeyDown}
          aria-label="Search labels"
        />
      </div>
      {filteredLabels.length === 0 && !showCreateOption && (
        <div className={styles.popEmpty}>No matching labels</div>
      )}
      {filteredLabels.map((label) => (
        <PickerRow
          key={label.label_id}
          active={selectedNames.includes(label.name)}
          onClick={() => onToggleLabel(label.name)}
        >
          <span
            className={styles.popLabelDot}
            style={{ backgroundColor: label.color }}
          />
          <span>{label.name}</span>
          <span className={styles.popSpacer} />
        </PickerRow>
      ))}
      {showCreateOption && (
        <>
          {filteredLabels.length > 0 && (
            <div className={styles.popDivider} role="separator" />
          )}
          <div className={styles.popSection}>Create new</div>
          <button
            type="button"
            className={styles.popRow}
            onClick={handleCreate}
          >
            <span
              className={styles.popLabelDot}
              style={{ backgroundColor: newLabelColor }}
            />
            <span className={styles.popCreateText}>
              Create &ldquo;{trimmed}&rdquo;
            </span>
            <span className={styles.popSpacer} />
          </button>
          <div className={styles.popPalette} role="radiogroup" aria-label="Label color">
            {LABEL_COLOR_PALETTE.map((color) => (
              <button
                key={color}
                type="button"
                role="radio"
                aria-checked={color === newLabelColor}
                aria-label={`Color ${color}`}
                className={`${styles.popSwatch}${
                  color === newLabelColor ? ` ${styles.popSwatchActive}` : ""
                }`}
                style={{ backgroundColor: color }}
                onClick={() => setNewLabelColor(color)}
              />
            ))}
          </div>
        </>
      )}
    </Picker>
  );
}
