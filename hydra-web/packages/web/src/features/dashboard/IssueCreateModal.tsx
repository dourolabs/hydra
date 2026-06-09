import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import type { IssueCreateModalInitial } from "./useIssueCreateModal";
import { Avatar, Button, Icons, Kbd, Picker, PickerRow, TypeChip } from "@hydra/ui";
import { DEFAULT_PROJECT_ID } from "@hydra/api";
import type { IssueType, LabelRecord, Principal, StatusKey } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useFormDraft } from "../../hooks/useFormDraft";
import { useFormModal } from "../../hooks/useFormModal";
import { useAuth } from "../auth/useAuth";
import { actorDisplayName } from "../../api/auth";
import { useLabels } from "../labels/useLabels";
import { useProjects, useProjectStatuses } from "../projects/useProjects";
import { StatusChip } from "../projects/StatusChip";
import { LABEL_COLOR_PALETTE } from "../../components/ColorPicker";
import styles from "./IssueCreateModal.module.css";

type PickerKey =
  | "type"
  | "assignee"
  | "repo"
  | "labels"
  | "project"
  | "status"
  | null;

const LEGACY_DEFAULT_STATUS_KEY: StatusKey = "open";

const ISSUE_TYPES: IssueType[] = ["task", "bug", "feature", "chore"];
const LABEL_PILL_MAX_INLINE = 2;

interface AssigneeView {
  name: string;
  kind: "agent" | "user";
}

function parseAssigneePath(path: string): Principal | null {
  if (!path) return null;
  if (path.startsWith("agents/")) {
    return { Agent: { name: path.slice("agents/".length) } };
  }
  if (path.startsWith("users/")) {
    return { User: { name: path.slice("users/".length) } };
  }
  return null;
}

function principalToView(p: Principal): AssigneeView | null {
  if ("User" in p) return { name: p.User.name, kind: "user" };
  if ("Agent" in p) return { name: p.Agent.name, kind: "agent" };
  return null;
}

export interface AssigneeGroups {
  agents: string[];
  users: string[];
}

interface IssueCreateModalProps {
  open: boolean;
  onClose: () => void;
  assignees: AssigneeGroups;
  // When the modal is opened from a scoped context (e.g. the IssuesBoard's
  // per-column "+ Add issue" button), the caller seeds the project and/or
  // status fields. These are applied on every open-transition so that opening
  // the modal scoped to a different column overrides any prior selection.
  initial?: IssueCreateModalInitial | null;
}

export function IssueCreateModal({
  open,
  onClose,
  assignees,
  initial,
}: IssueCreateModalProps) {
  const { user } = useAuth();
  const { data: repos } = useRepositories();
  const { data: labels } = useLabels();
  const { data: projects } = useProjects();
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
  // Assignee is stored as a Principal path (`agents/<name>` / `users/<name>`)
  // so the picker can derive both the display name and the wire `kind` without
  // round-tripping through the two source lists.
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
  // Empty string = unselected — falls back to the seeded default project
  // (`j-defaul`) at submit time so the create request still carries a
  // populated project_id.
  const [projectId, setProjectId, clearProjectIdDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:projectId",
    "",
  );
  // Empty string = unselected; falls back to `LEGACY_DEFAULT_STATUS_KEY` at
  // submit time so the wire body stays stable when the user doesn't touch the
  // status picker.
  const [status, setStatus, clearStatusDraft] = useFormDraft<StatusKey>(
    "hydra:draft:issue-create-modal:status",
    "",
  );

  // Status options follow the selected project (or the seeded default
  // project when unselected). Same hook the IssueUpdateModal uses.
  const { data: projectStatuses } = useProjectStatuses(projectId || null);

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

  // When the modal opens with seeded `initial.projectId` / `initial.status`
  // (the IssuesBoard "+ Add issue" path), apply them over the persisted
  // draft. We deliberately don't touch title/description here — opening
  // scoped to a column shouldn't blow away in-progress text.
  const wasOpenRef = useRef(false);
  useEffect(() => {
    const justOpened = open && !wasOpenRef.current;
    wasOpenRef.current = open;
    if (!justOpened || !initial) return;
    if (initial.projectId !== undefined) setProjectId(initial.projectId);
    if (initial.status !== undefined) setStatus(initial.status);
  }, [open, initial, setProjectId, setStatus]);

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
    setProjectId("");
    setStatus("");
    clearTitleDraft();
    clearDescriptionDraft();
    clearIssueTypeDraft();
    clearAssigneeDraft();
    clearRepoNameDraft();
    clearLabelNamesDraft();
    clearProjectIdDraft();
    clearStatusDraft();
  }, [
    setTitle,
    setDescription,
    setIssueType,
    setAssignee,
    setRepoName,
    setLabelNames,
    setProjectId,
    setStatus,
    clearTitleDraft,
    clearDescriptionDraft,
    clearIssueTypeDraft,
    clearAssigneeDraft,
    clearRepoNameDraft,
    clearLabelNamesDraft,
    clearProjectIdDraft,
    clearStatusDraft,
  ]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    {
      title: string;
      description: string;
      creator: string;
      type: IssueType;
      status: StatusKey;
      assignee?: Principal;
      repoName?: string;
      labelNames?: string[];
      projectId?: string;
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
          status: params.status,
          project_id: params.projectId || DEFAULT_PROJECT_ID,
          dependencies: [],
          patches: [],
          ...(params.assignee && { assignee: params.assignee }),
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
    const assigneePrincipal = parseAssigneePath(assignee);
    // Submit-time fallback: an explicit pick wins; otherwise the legacy
    // `"open"` keeps the wire body stable for any project whose status fetch
    // hasn't resolved yet.
    const submitStatus: StatusKey = status || LEGACY_DEFAULT_STATUS_KEY;
    mutation.mutate({
      title: title.trim(),
      description: desc,
      creator: currentUsername,
      type: issueType,
      status: submitStatus,
      ...(assigneePrincipal && { assignee: assigneePrincipal }),
      ...(repoName && { repoName }),
      ...(labelNames.length > 0 && { labelNames }),
      ...(projectId && { projectId }),
    });
  }, [
    title,
    description,
    currentUsername,
    issueType,
    assignee,
    repoName,
    labelNames,
    projectId,
    status,
    projectStatuses,
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

  const assigneeView: AssigneeView | null = useMemo(() => {
    const parsed = parseAssigneePath(assignee);
    return parsed ? principalToView(parsed) : null;
  }, [assignee]);

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
  const projectEntries = projects ?? [];
  const statusEntries = projectStatuses?.statuses ?? [];
  const selectedProject = projectEntries.find(
    (p) => p.project_id === projectId,
  );
  const selectedStatusDef = statusEntries.find((s) => s.key === status);
  const effectiveStatusKey = status || LEGACY_DEFAULT_STATUS_KEY;
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
                assigneeView ? (
                  <span className={styles.pillContent}>
                    <Avatar
                      name={assigneeView.name}
                      kind={assigneeView.kind === "agent" ? "agent" : "human"}
                      size="md"
                    />
                    <span>{assigneeView.name}</span>
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
              {assignees.agents.length > 0 && (
                <>
                  <div className={styles.popSection}>Agents</div>
                  {assignees.agents.map((name) => {
                    const path = `agents/${name}`;
                    return (
                      <PickerRow
                        key={path}
                        active={assignee === path}
                        onClick={() => {
                          setAssignee(path);
                          setPicker(null);
                        }}
                      >
                        <Avatar name={name} kind="agent" size="md" />
                        <span>{name}</span>
                        <span className={styles.popSpacer} />
                      </PickerRow>
                    );
                  })}
                </>
              )}
              {assignees.users.length > 0 && (
                <>
                  <div className={styles.popSection}>Users</div>
                  {assignees.users.map((name) => {
                    const path = `users/${name}`;
                    return (
                      <PickerRow
                        key={path}
                        active={assignee === path}
                        onClick={() => {
                          setAssignee(path);
                          setPicker(null);
                        }}
                      >
                        <Avatar name={name} kind="human" size="md" />
                        <span>{name}</span>
                        <span className={styles.popSpacer} />
                      </PickerRow>
                    );
                  })}
                </>
              )}
            </Picker>

            <div data-testid="issue-create-project-picker">
              <Picker
                label="Project"
                open={picker === "project"}
                onToggle={() => setPicker(picker === "project" ? null : "project")}
                wide
                value={
                  selectedProject ? (
                    <span className={styles.pillContent}>
                      <code className={styles.pillCode}>
                        {selectedProject.project.key}
                      </code>
                    </span>
                  ) : (
                    <span className={styles.pillEmpty}>Default</span>
                  )
                }
              >
                <PickerRow
                  active={!projectId}
                  onClick={() => {
                    setProjectId("");
                    // Reset the explicit status pick so the picker falls back
                    // to LEGACY_DEFAULT_STATUS_KEY on the next render.
                    setStatus("");
                    setPicker(null);
                  }}
                >
                  <span className={styles.pillEmpty}>Default</span>
                  <span className={styles.popSpacer} />
                </PickerRow>
                {projectEntries.map((p) => (
                  <PickerRow
                    key={p.project_id}
                    active={projectId === p.project_id}
                    onClick={() => {
                      setProjectId(p.project_id);
                      // Reset to "" so the next render re-derives the picker's
                      // default from the new project's status list.
                      setStatus("");
                      setPicker(null);
                    }}
                  >
                    <code className={styles.popCode}>{p.project.key}</code>
                    <span className={styles.popSub}>{p.project.name}</span>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                ))}
              </Picker>
            </div>

            <div data-testid="issue-create-status-picker">
              <Picker
                label="Status"
                open={picker === "status"}
                onToggle={() => setPicker(picker === "status" ? null : "status")}
                wide
                value={
                  selectedStatusDef ? (
                    <StatusChip status={selectedStatusDef} />
                  ) : (
                    <span>{effectiveStatusKey}</span>
                  )
                }
              >
                {statusEntries.length === 0 ? (
                  <div className={styles.popEmpty}>No statuses</div>
                ) : (
                  statusEntries.map((s) => (
                    <PickerRow
                      key={s.key}
                      active={effectiveStatusKey === s.key}
                      onClick={() => {
                        setStatus(s.key);
                        setPicker(null);
                      }}
                    >
                      <StatusChip status={s} />
                      <span className={styles.popSpacer} />
                    </PickerRow>
                  ))
                )}
              </Picker>
            </div>

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
