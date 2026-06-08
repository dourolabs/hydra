import { useCallback, useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Modal, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type {
  DocumentPath,
  IssueSummaryRecord,
  ListProjectsResponse,
  Principal,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useAgents } from "../../hooks/useAgents";
import { useUsers } from "../../hooks/useUsers";
import { ColorPicker, LABEL_COLOR_PALETTE } from "../../components/ColorPicker";
import {
  principalKind,
  principalToPath,
  pathToPrincipal,
  type AssignKind,
} from "./principalAssign";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticUpsert,
} from "./projectCache";
import { blankStatus, validateStatusKey } from "./statusDefaults";
import styles from "./StatusSettingsModal.module.css";

export interface StatusSettingsModalProps {
  open: boolean;
  onClose: () => void;
  projectRecord: ProjectRecord;
  /** Defaults to "edit". In "new" mode `statusKey`/`issueCount` are ignored. */
  mode?: "edit" | "new";
  /** Required in edit mode; ignored in new mode. */
  statusKey?: string;
  /** Required in edit mode; ignored in new mode. */
  issueCount?: number;
}

export function StatusSettingsModal({
  open,
  onClose,
  projectRecord,
  mode: modeProp,
  statusKey: statusKeyProp,
  issueCount: issueCountProp,
}: StatusSettingsModalProps) {
  const mode: "edit" | "new" = modeProp ?? "edit";
  const statusKey = statusKeyProp ?? "";
  const issueCount = issueCountProp ?? 0;
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  const statuses = projectRecord.project.statuses;
  const initialStatus = useMemo(() => {
    if (mode === "new") return null;
    const i = statuses.findIndex((s) => s.key === statusKey);
    return i >= 0 ? { status: statuses[i], index: i } : null;
  }, [mode, statuses, statusKey]);
  const index = initialStatus?.index ?? -1;

  const [draft, setDraft] = useState<StatusDefinition | null>(() =>
    mode === "new" ? blankStatus(statuses.length) : initialStatus?.status ?? null,
  );
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [moveTargetKey, setMoveTargetKey] = useState<string>("");
  const [moveProgress, setMoveProgress] = useState<{
    current: number;
    total: number;
  } | null>(null);

  // Resync local draft whenever the modal is opened against a different
  // status (gear click on another column reuses the same component instance).
  useEffect(() => {
    if (!open) return;
    setDraft(
      mode === "new" ? blankStatus(statuses.length) : initialStatus?.status ?? null,
    );
    setConfirmingDelete(false);
    setMoveProgress(null);
  }, [open, mode, statuses.length, initialStatus]);

  const projectId = projectRecord.project_id;

  const saveMutation = useMutation({
    mutationFn: async (nextStatuses: StatusDefinition[]) => {
      return apiClient.updateProject(projectId, {
        project: { ...projectRecord.project, statuses: nextStatuses },
      });
    },
    onMutate: async (nextStatuses) => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const nextProject = {
          ...projectRecord.project,
          statuses: nextStatuses,
        };
        const next: ListProjectsResponse = {
          projects: applyOptimisticUpsert(previous.projects, projectId, nextProject),
        };
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, next);
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(PROJECTS_QUERY_KEY, context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to update status",
        "error",
      );
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
    },
  });

  const onlyStatus = statuses.length <= 1;
  const hasIssues = issueCount > 0;
  const canDelete = !onlyStatus;
  const deleteTooltip = onlyStatus
    ? "Cannot delete the only status"
    : hasIssues
    ? `Will move ${issueCount} open issue(s) to a sibling status`
    : "";

  // Neighbor statuses available for the bulk-move (excludes the to-delete one).
  const moveOptions: SelectOption[] = useMemo(
    () =>
      statuses
        .filter((_, i) => i !== index)
        .map((s) => ({ value: s.key as string, label: s.label || (s.key as string) })),
    [statuses, index],
  );

  // Default neighbor: left of the to-delete column, or the right neighbor when
  // to-delete is the leftmost. `index === -1` ("not found") falls back to "".
  const defaultMoveTargetKey = useMemo(() => {
    if (index < 0) return "";
    const left = statuses[index - 1];
    if (left) return left.key as string;
    const right = statuses[index + 1];
    return right ? (right.key as string) : "";
  }, [statuses, index]);

  // Reset the move-target selection whenever the modal re-targets a status or
  // the user re-enters the confirming-delete substep — keeps the default
  // neighbor in sync with the current `index`.
  useEffect(() => {
    if (!confirmingDelete) return;
    setMoveTargetKey(defaultMoveTargetKey);
  }, [confirmingDelete, defaultMoveTargetKey]);

  const existingKeysExceptDraft = useMemo(
    () => new Set(statuses.map((s) => s.key as string)),
    [statuses],
  );

  const newModeError = useMemo(() => {
    if (mode !== "new" || !draft) return null;
    const keyErr = validateStatusKey(draft.key, existingKeysExceptDraft);
    if (keyErr) return keyErr;
    if (!draft.label.trim()) return "Status label is required";
    return null;
  }, [mode, draft, existingKeysExceptDraft]);

  const handleSave = useCallback(() => {
    if (!draft) return;
    const trimmedPromptPath = draft.prompt_path?.trim() ?? "";
    const normalized: StatusDefinition = {
      ...draft,
      key: draft.key.trim(),
      label: draft.label.trim(),
      prompt_path: trimmedPromptPath ? trimmedPromptPath : null,
    };
    if (mode === "new") {
      if (newModeError) {
        addToast(newModeError, "error");
        return;
      }
      const next = [...statuses, normalized];
      saveMutation.mutate(next, {
        onSuccess: () => {
          addToast("Status added", "success");
          onClose();
        },
      });
      return;
    }
    if (index < 0) return;
    const next = statuses.map((s, i) => (i === index ? normalized : s));
    saveMutation.mutate(next, {
      onSuccess: () => {
        addToast("Status updated", "success");
        onClose();
      },
    });
  }, [draft, mode, newModeError, index, statuses, saveMutation, addToast, onClose]);

  // Move stays inside the modal: persist the swap but do NOT close, so the
  // user can keep nudging the column without reopening the gear each time.
  const handleMove = useCallback(
    (delta: number) => {
      if (index < 0) return;
      const target = index + delta;
      if (target < 0 || target >= statuses.length) return;
      const next = [...statuses];
      const tmp = next[index];
      next[index] = next[target];
      next[target] = tmp;
      saveMutation.mutate(next, {
        onSuccess: () => addToast("Status moved", "success"),
      });
    },
    [index, statuses, saveMutation, addToast],
  );

  const handleDelete = useCallback(() => {
    if (!canDelete || index < 0) return;
    const next = statuses.filter((_, i) => i !== index);
    saveMutation.mutate(next, {
      onSuccess: () => {
        addToast("Status deleted", "success");
        onClose();
      },
    });
  }, [canDelete, index, statuses, saveMutation, addToast, onClose]);

  // Bulk-move every issue at the to-delete status onto `moveTargetKey`, then
  // drop the status from the project's `statuses`. Errors halt the move
  // before the project save fires so we never orphan a status with a partial
  // migration. We do the per-issue work outside of `saveMutation` because
  // (a) it needs sequential progress reporting and (b) we also need to
  // possibly re-point `default_status_key`, which `saveMutation` doesn't
  // touch.
  const moveAndDeleteMutation = useMutation({
    mutationFn: async (targetKey: string) => {
      if (index < 0) throw new Error("Status not found in project");

      // 1) Enumerate every issue at the to-delete status (paginated).
      const ids: string[] = [];
      let cursor: string | null = null;
      let more = true;
      while (more) {
        const resp = await apiClient.listIssues({
          project_id: projectId,
          status: statusKey,
          limit: null,
          ...(cursor ? { cursor } : {}),
        });
        for (const rec of resp.issues as IssueSummaryRecord[]) {
          ids.push(rec.issue_id as string);
        }
        cursor = resp.next_cursor ?? null;
        more = !!cursor;
      }

      // 2) Sequentially patch each issue's status to the neighbor. Refetch
      //    the full Issue body first so we don't clobber the description /
      //    session_settings with the truncated summary shape.
      setMoveProgress({ current: 0, total: ids.length });
      for (let i = 0; i < ids.length; i++) {
        const id = ids[i];
        setMoveProgress({ current: i + 1, total: ids.length });
        try {
          const record = await apiClient.getIssue(id);
          await apiClient.updateIssue(id, {
            issue: { ...record.issue, status: targetKey },
            session_id: null,
          });
        } catch (err) {
          const reason = err instanceof Error ? err.message : "request failed";
          throw new Error(
            `Move halted at issue ${id}: ${reason}. No statuses were deleted.`,
          );
        }
      }

      // 3) Optimistic project save: drop the status, retarget the default
      //    status key if it was the deleted one. Prefer the left neighbor;
      //    fall back to the right neighbor (i.e. nextStatuses[0]) when the
      //    deleted column was the leftmost.
      const nextStatuses = statuses.filter((_, i) => i !== index);
      const wasDefault =
        projectRecord.project.default_status_key === statusKey;
      let nextDefaultKey = projectRecord.project.default_status_key;
      if (wasDefault) {
        const leftKey = nextStatuses[index - 1]?.key;
        nextDefaultKey = leftKey ?? nextStatuses[0]?.key ?? nextDefaultKey;
      }
      const nextProject = {
        ...projectRecord.project,
        statuses: nextStatuses,
        default_status_key: nextDefaultKey,
      };

      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, {
          projects: applyOptimisticUpsert(previous.projects, projectId, nextProject),
        });
      }
      try {
        return await apiClient.updateProject(projectId, { project: nextProject });
      } catch (err) {
        if (previous) {
          queryClient.setQueryData(PROJECTS_QUERY_KEY, previous);
        }
        throw err;
      }
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
      queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
      addToast(`Moved ${issueCount} issue(s) and deleted status`, "success");
      setMoveProgress(null);
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to move issues",
        "error",
      );
      setMoveProgress(null);
    },
  });

  const handleMoveAndDelete = useCallback(() => {
    if (!canDelete || index < 0 || !hasIssues) return;
    if (!moveTargetKey) {
      addToast("Pick a status to move issues to", "error");
      return;
    }
    moveAndDeleteMutation.mutate(moveTargetKey);
  }, [
    canDelete,
    index,
    hasIssues,
    moveTargetKey,
    moveAndDeleteMutation,
    addToast,
  ]);

  if (!draft || (mode === "edit" && index < 0)) {
    return (
      <Modal open={open} onClose={onClose} title="Status settings">
        <div className={styles.body}>
          <p className={styles.error}>Status "{statusKey}" not found in project.</p>
          <div className={styles.actions}>
            <Button variant="secondary" size="md" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
      </Modal>
    );
  }

  const title =
    mode === "new"
      ? `New status · ${projectRecord.project.name}`
      : `Status — ${draft.label || draft.key}`;

  return (
    <Modal open={open} onClose={onClose} title={title}>
      <StatusForm
        draft={draft}
        setDraft={setDraft}
        agents={agents?.map((a) => a.name) ?? []}
        users={users?.map((u) => u.username) ?? []}
        agentsLoaded={agents !== undefined}
        usersLoaded={users !== undefined}
        index={index}
        count={statuses.length}
        onMove={handleMove}
        saving={saveMutation.isPending}
        mode={mode}
      />

      {mode === "new" && newModeError && (
        <span className={styles.error} data-testid="status-settings-new-error">
          {newModeError}
        </span>
      )}

      <div className={styles.actions} data-testid="status-settings-actions">
        <div className={styles.actionsLeft}>
          {mode === "edit" &&
            (confirmingDelete ? (
              hasIssues ? (
                <div
                  className={styles.moveBlock}
                  data-testid="status-settings-move-block"
                >
                  <label className={styles.label}>
                    Move {issueCount} issue(s) to:
                    <Select
                      options={moveOptions}
                      value={moveTargetKey}
                      onChange={(e) => setMoveTargetKey(e.target.value)}
                      data-testid="status-settings-move-target"
                    />
                  </label>
                  <div className={styles.moveActions}>
                    {moveProgress && moveProgress.total > 0 && (
                      <span
                        className={styles.label}
                        data-testid="status-settings-move-progress"
                      >
                        Moving {moveProgress.current} of {moveProgress.total}…
                      </span>
                    )}
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => setConfirmingDelete(false)}
                      disabled={moveAndDeleteMutation.isPending}
                    >
                      Cancel
                    </Button>
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={handleMoveAndDelete}
                      disabled={
                        moveAndDeleteMutation.isPending || !moveTargetKey
                      }
                      data-testid="status-settings-move-confirm"
                    >
                      {moveAndDeleteMutation.isPending
                        ? "Moving…"
                        : `Move ${issueCount} and delete`}
                    </Button>
                  </div>
                </div>
              ) : (
                <>
                  <span className={styles.label}>Delete this status?</span>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setConfirmingDelete(false)}
                    disabled={saveMutation.isPending}
                  >
                    Cancel
                  </Button>
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={handleDelete}
                    disabled={saveMutation.isPending}
                    data-testid="status-settings-delete-confirm"
                  >
                    {saveMutation.isPending ? "Deleting…" : "Confirm delete"}
                  </Button>
                </>
              )
            ) : (
              <button
                type="button"
                className={`${styles.miniButton} ${styles.miniButtonDanger}`}
                onClick={() => setConfirmingDelete(true)}
                disabled={!canDelete || saveMutation.isPending}
                title={deleteTooltip || undefined}
                data-testid="status-settings-delete"
              >
                Delete status
              </button>
            ))}
        </div>
        <div className={styles.actionsRight}>
          <Button
            variant="secondary"
            size="md"
            onClick={onClose}
            disabled={saveMutation.isPending || moveAndDeleteMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSave}
            disabled={
              saveMutation.isPending ||
              moveAndDeleteMutation.isPending ||
              (mode === "new" && !!newModeError)
            }
            data-testid="status-settings-save"
          >
            {saveButtonLabel(mode, saveMutation.isPending)}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function saveButtonLabel(mode: "edit" | "new", pending: boolean): string {
  if (mode === "new") return pending ? "Adding…" : "Add status";
  return pending ? "Saving…" : "Save";
}

interface StatusFormProps {
  draft: StatusDefinition;
  setDraft: (next: StatusDefinition) => void;
  agents: string[];
  users: string[];
  agentsLoaded: boolean;
  usersLoaded: boolean;
  index: number;
  count: number;
  onMove: (delta: number) => void;
  saving: boolean;
  mode: "edit" | "new";
}

function StatusForm({
  draft,
  setDraft,
  agents,
  users,
  agentsLoaded,
  usersLoaded,
  index,
  count,
  onMove,
  saving,
  mode,
}: StatusFormProps) {
  const onEnter = draft.on_enter ?? null;
  const assignKind = principalKind(onEnter?.assign_to ?? null);
  const principalPath = onEnter?.assign_to ? principalToPath(onEnter.assign_to) : "";
  const external = onEnter?.assign_to && "External" in onEnter.assign_to
    ? onEnter.assign_to.External
    : null;
  const attachForm = onEnter?.attach_form ?? "";

  const patch = (p: Partial<StatusDefinition>) => setDraft({ ...draft, ...p });

  const userOptions: SelectOption[] = useMemo(
    () => [
      { value: "", label: "— select user —" },
      ...users.map((u) => ({ value: `users/${u}`, label: u })),
    ],
    [users],
  );
  const agentOptions: SelectOption[] = useMemo(
    () => [
      { value: "", label: "— select agent —" },
      ...agents.map((a) => ({ value: `agents/${a}`, label: a })),
    ],
    [agents],
  );
  const hasUsers = usersLoaded && users.length > 0;
  const hasAgents = agentsLoaded && agents.length > 0;
  const kindOptions: SelectOption[] = useMemo(
    () => [
      { value: "none", label: "— none —" },
      { value: "user", label: hasUsers ? "User" : "User (none available)" },
      { value: "agent", label: hasAgents ? "Agent" : "Agent (none available)" },
      { value: "external", label: "External" },
    ],
    [hasUsers, hasAgents],
  );

  const setAssign = (next: Principal | null) => {
    const nextForm = onEnter?.attach_form ?? null;
    if (!next && !nextForm) {
      patch({ on_enter: null });
      return;
    }
    patch({ on_enter: { assign_to: next, attach_form: nextForm } });
  };

  const setAttachForm = (raw: string) => {
    const nextForm = raw ? (raw as DocumentPath) : null;
    const nextAssign = onEnter?.assign_to ?? null;
    if (!nextAssign && !nextForm) {
      patch({ on_enter: null });
      return;
    }
    patch({ on_enter: { assign_to: nextAssign, attach_form: nextForm } });
  };

  const setKind = (kind: AssignKind) => {
    if (kind === "none") return setAssign(null);
    // Don't seed an empty Principal name — Principal::{User,Agent}.name must
    // always be a real handle. If the list isn't loaded yet (or is empty),
    // leave the existing assignment alone so the user can pick once it loads.
    if (kind === "user") {
      if (!hasUsers) return;
      return setAssign({ User: { name: users[0] } });
    }
    if (kind === "agent") {
      if (!hasAgents) return;
      return setAssign({ Agent: { name: agents[0] } });
    }
    setAssign({
      External: { system: external?.system ?? "", username: external?.username ?? "" },
    });
  };

  return (
    <div className={styles.body} data-testid="status-settings-form">
      {mode === "edit" && (
        <div className={styles.header}>
          <button
            type="button"
            className={styles.miniButton}
            onClick={() => onMove(-1)}
            disabled={index === 0 || saving}
            aria-label="Move left"
            data-testid="status-settings-move-left"
          >
            ← Move left
          </button>
          <button
            type="button"
            className={styles.miniButton}
            onClick={() => onMove(1)}
            disabled={index === count - 1 || saving}
            aria-label="Move right"
            data-testid="status-settings-move-right"
          >
            Move right →
          </button>
          <span style={{ flex: 1 }} />
          <span className={styles.label}>
            Position {index + 1} of {count}
          </span>
        </div>
      )}

      <div className={styles.statusInputs}>
        {mode === "new" ? (
          <Input
            label="Key"
            value={draft.key}
            onChange={(e) => patch({ key: e.target.value })}
            placeholder="in-progress"
            required
            data-testid="status-settings-key"
          />
        ) : (
          <div>
            <Input
              label="Key"
              value={draft.key}
              disabled
              data-testid="status-settings-key"
            />
            <span className={styles.readOnlyNote}>
              Key rename is not yet supported (orphans live issues).
            </span>
          </div>
        )}
        <Input
          label="Label"
          value={draft.label}
          onChange={(e) => patch({ label: e.target.value })}
          placeholder="In progress"
          required
          data-testid="status-settings-label"
        />
      </div>

      <div className={styles.row}>
        <label className={styles.label}>Color</label>
        <ColorPicker
          value={draft.color}
          onChange={(color) => patch({ color })}
          palette={LABEL_COLOR_PALETTE}
          allowCustom
        />
      </div>

      <div className={styles.flagRow}>
        <label>
          <input
            type="checkbox"
            checked={draft.unblocks_parents}
            onChange={(e) => patch({ unblocks_parents: e.target.checked })}
          />
          Unblocks parents (terminal)
        </label>
        <label>
          <input
            type="checkbox"
            checked={draft.unblocks_dependents}
            onChange={(e) => patch({ unblocks_dependents: e.target.checked })}
          />
          Unblocks dependents
        </label>
        <label>
          <input
            type="checkbox"
            checked={draft.cascades_to_children}
            onChange={(e) => patch({ cascades_to_children: e.target.checked })}
          />
          Cascades to children
        </label>
        <label>
          <input
            type="checkbox"
            checked={draft.interactive ?? false}
            onChange={(e) => patch({ interactive: e.target.checked })}
            data-testid="status-settings-interactive"
          />
          Interactive
        </label>
      </div>

      <div className={styles.onEnter}>
        <span className={styles.onEnterTitle}>On enter</span>
        <Select
          label="Assign to"
          options={kindOptions}
          value={assignKind}
          onChange={(e) => setKind(e.target.value as AssignKind)}
          data-testid="status-settings-assign-kind"
        />
        {assignKind === "user" && (
          <Select
            label="User"
            options={userOptions}
            value={principalPath}
            onChange={(e) => setAssign(pathToPrincipal(e.target.value))}
          />
        )}
        {assignKind === "agent" && (
          <Select
            label="Agent"
            options={agentOptions}
            value={principalPath}
            onChange={(e) => setAssign(pathToPrincipal(e.target.value))}
          />
        )}
        {assignKind === "external" && (
          <div className={styles.statusInputs}>
            <Input
              label="System"
              value={external?.system ?? ""}
              onChange={(e) =>
                setAssign({
                  External: {
                    system: e.target.value,
                    username: external?.username ?? "",
                  },
                })
              }
              placeholder="github"
            />
            <Input
              label="Username"
              value={external?.username ?? ""}
              onChange={(e) =>
                setAssign({
                  External: {
                    system: external?.system ?? "",
                    username: e.target.value,
                  },
                })
              }
              placeholder="jayantk"
            />
          </div>
        )}
        <Input
          label="Attach form"
          value={attachForm}
          onChange={(e) => setAttachForm(e.target.value)}
          placeholder="/forms/review.yaml"
        />
      </div>

      <Input
        label="Prompt path"
        value={draft.prompt_path ?? ""}
        onChange={(e) => patch({ prompt_path: e.target.value })}
        placeholder="/projects/<key>/statuses/<status-key>.md"
        data-testid="status-settings-prompt-path"
      />
    </div>
  );
}
