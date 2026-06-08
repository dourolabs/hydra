import { useCallback, useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Avatar,
  Button,
  Input,
  Modal,
  Picker,
  PickerRow,
  Select,
  Textarea,
} from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type {
  DocumentPath,
  IssueSummaryRecord,
  ListProjectsResponse,
  Principal,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";
import { ApiError, apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useAgents } from "../../hooks/useAgents";
import { useUsers } from "../../hooks/useUsers";
import { ColorPicker, LABEL_COLOR_PALETTE } from "../../components/ColorPicker";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticUpsert,
} from "./projectCache";
import { blankStatus, slugifyStatusKey } from "./statusDefaults";
import { PromptDocumentEditor } from "./PromptDocumentEditor";
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

export function StatusSettingsModal(props: StatusSettingsModalProps) {
  const mode: "edit" | "new" = props.mode ?? "edit";
  if (mode === "new") {
    return (
      <Modal
        open={props.open}
        onClose={props.onClose}
        title={`New status · ${props.projectRecord.project.name}`}
      >
        <AddStatusForm
          onClose={props.onClose}
          projectRecord={props.projectRecord}
        />
      </Modal>
    );
  }
  return <EditStatusModal {...props} />;
}

function EditStatusModal({
  open,
  onClose,
  projectRecord,
  statusKey: statusKeyProp,
  issueCount: issueCountProp,
}: StatusSettingsModalProps) {
  const statusKey = statusKeyProp ?? "";
  const issueCount = issueCountProp ?? 0;
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  const statuses = projectRecord.project.statuses;
  const initialStatus = useMemo(() => {
    const i = statuses.findIndex((s) => s.key === statusKey);
    return i >= 0 ? { status: statuses[i], index: i } : null;
  }, [statuses, statusKey]);
  const index = initialStatus?.index ?? -1;

  const [draft, setDraft] = useState<StatusDefinition | null>(
    () => initialStatus?.status ?? null,
  );
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [moveTargetKey, setMoveTargetKey] = useState<string>("");
  const [moveProgress, setMoveProgress] = useState<{
    current: number;
    total: number;
  } | null>(null);
  const [promptExpanded, setPromptExpanded] = useState(false);

  // Resync local draft whenever the modal is opened against a different
  // status (gear click on another column reuses the same component instance).
  useEffect(() => {
    if (!open) return;
    setDraft(initialStatus?.status ?? null);
    setConfirmingDelete(false);
    setMoveProgress(null);
    setPromptExpanded(false);
  }, [open, initialStatus]);

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

  const handleSave = useCallback(() => {
    if (!draft || index < 0) return;
    const trimmedPromptPath = draft.prompt_path?.trim() ?? "";
    const normalized: StatusDefinition = {
      ...draft,
      key: draft.key.trim(),
      label: draft.label.trim(),
      prompt_path: trimmedPromptPath ? trimmedPromptPath : null,
    };
    const next = statuses.map((s, i) => (i === index ? normalized : s));
    saveMutation.mutate(next, {
      onSuccess: () => {
        addToast("Status updated", "success");
        onClose();
      },
    });
  }, [draft, index, statuses, saveMutation, addToast, onClose]);

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
  // migration. We do the per-issue work outside of `saveMutation` because it
  // needs sequential progress reporting.
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

      // 3) Optimistic project save: drop the status from the list.
      const nextStatuses = statuses.filter((_, i) => i !== index);
      const nextProject = {
        ...projectRecord.project,
        statuses: nextStatuses,
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

  if (!draft || index < 0) {
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

  const title = `Status — ${draft.label || draft.key}`;

  return (
    <Modal open={open} onClose={onClose} title={title}>
      <StatusForm
        draft={draft}
        setDraft={setDraft}
        agents={agents?.map((a) => a.name) ?? []}
        users={users?.map((u) => u.username) ?? []}
        projectKey={projectRecord.project.key as string}
        statusKeyForDefaultPath={statusKey}
        promptExpanded={promptExpanded}
        onTogglePromptExpanded={() => setPromptExpanded((v) => !v)}
      />

      <div className={styles.actions} data-testid="status-settings-actions">
        <div className={styles.actionsLeft}>
          {confirmingDelete ? (
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
          )}
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
            disabled={saveMutation.isPending || moveAndDeleteMutation.isPending}
            data-testid="status-settings-save"
          >
            {saveMutation.isPending ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

interface StatusFormProps {
  draft: StatusDefinition;
  setDraft: (next: StatusDefinition) => void;
  agents: string[];
  users: string[];
  projectKey: string;
  statusKeyForDefaultPath: string;
  promptExpanded: boolean;
  onTogglePromptExpanded: () => void;
}

function StatusForm({
  draft,
  setDraft,
  agents,
  users,
  projectKey,
  statusKeyForDefaultPath,
  promptExpanded,
  onTogglePromptExpanded,
}: StatusFormProps) {
  const onEnter = draft.on_enter ?? null;
  const principal = onEnter?.assign_to ?? null;
  const attachForm = onEnter?.attach_form ?? "";
  const [assigneePickerOpen, setAssigneePickerOpen] = useState(false);

  const patch = (p: Partial<StatusDefinition>) => setDraft({ ...draft, ...p });

  // The new flat picker doesn't model External, so legacy External
  // assignments render as "Unassigned" in the trigger pill until the user
  // re-picks (the underlying value stays put unless they pick a new row).
  const assigneeView: { name: string; kind: "agent" | "user" } | null =
    useMemo(() => {
      if (!principal) return null;
      if ("Agent" in principal) return { name: principal.Agent.name, kind: "agent" };
      if ("User" in principal) return { name: principal.User.name, kind: "user" };
      return null;
    }, [principal]);

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

  return (
    <div className={styles.body} data-testid="status-settings-form">
      <div className={styles.statusInputs}>
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
        <div data-testid="status-settings-assignee">
          <Picker
            label="Assign to"
            open={assigneePickerOpen}
            onToggle={() => setAssigneePickerOpen((v) => !v)}
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
              active={!principal}
              onClick={() => {
                setAssign(null);
                setAssigneePickerOpen(false);
              }}
            >
              <span className={styles.pillEmpty}>Unassigned</span>
              <span className={styles.popSpacer} />
            </PickerRow>
            {agents.length > 0 && (
              <>
                <div className={styles.popSection}>Agents</div>
                {agents.map((name) => (
                  <PickerRow
                    key={`agents/${name}`}
                    active={
                      !!principal &&
                      "Agent" in principal &&
                      principal.Agent.name === name
                    }
                    onClick={() => {
                      setAssign({ Agent: { name } });
                      setAssigneePickerOpen(false);
                    }}
                  >
                    <Avatar name={name} kind="agent" size="md" />
                    <span>{name}</span>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                ))}
              </>
            )}
            {users.length > 0 && (
              <>
                <div className={styles.popSection}>Users</div>
                {users.map((name) => (
                  <PickerRow
                    key={`users/${name}`}
                    active={
                      !!principal &&
                      "User" in principal &&
                      principal.User.name === name
                    }
                    onClick={() => {
                      setAssign({ User: { name } });
                      setAssigneePickerOpen(false);
                    }}
                  >
                    <Avatar name={name} kind="human" size="md" />
                    <span>{name}</span>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                ))}
              </>
            )}
          </Picker>
        </div>
        <Input
          label="Attach form"
          value={attachForm}
          onChange={(e) => setAttachForm(e.target.value)}
          placeholder="/forms/review.yaml"
        />
      </div>

      <PromptDocumentEditor
        path={draft.prompt_path ?? null}
        defaultPath={`/projects/${projectKey || "<key>"}/statuses/${
          statusKeyForDefaultPath.trim() || "<status-key>"
        }.md`}
        onPathChange={(value) => patch({ prompt_path: value })}
        expanded={promptExpanded}
        onToggleExpanded={onTogglePromptExpanded}
        label="Prompt path"
        placeholder="/projects/<key>/statuses/<status-key>.md"
        testId="status-settings-prompt-path"
      />
    </div>
  );
}

interface AddStatusFormProps {
  onClose: () => void;
  projectRecord: ProjectRecord;
}

// Minimal "Add status" flow: a single Name input drives both the user-visible
// label and a slugified key, and an inline markdown editor writes the prompt
// document. The status's `prompt_path` is auto-generated from project + slug;
// users never see or edit it. The advanced fields (color, on-enter, cascades)
// stay reachable through the gear's edit modal after the column lands.
function AddStatusForm({ onClose, projectRecord }: AddStatusFormProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const statuses = projectRecord.project.statuses;
  const projectKey = projectRecord.project.key as string;
  const projectId = projectRecord.project_id;

  const [name, setName] = useState("");
  const [body, setBody] = useState("");

  const key = useMemo(() => slugifyStatusKey(name), [name]);
  const existingKeys = useMemo(
    () => new Set(statuses.map((s) => s.key as string)),
    [statuses],
  );
  const promptPath = key ? `/projects/${projectKey}/statuses/${key}.md` : "";

  const nameError = useMemo(() => {
    if (!name.trim()) return null;
    if (!key) return "Status name must include a letter or digit";
    if (existingKeys.has(key)) {
      return `Status '${key}' already exists in this project`;
    }
    return null;
  }, [name, key, existingKeys]);

  const saveMutation = useMutation({
    mutationFn: async (status: StatusDefinition) => {
      const trimmedBody = body.trim();
      if (trimmedBody) {
        // Upsert the prompt doc at the auto path. We check first because
        // re-creating a previously-deleted status under the same key would
        // collide on the unique-path constraint server-side.
        try {
          const existing = await apiClient.getDocumentByPath(status.prompt_path as string);
          await apiClient.updateDocument(existing.document_id, {
            document: {
              ...existing.document,
              body_markdown: body,
              path: status.prompt_path as DocumentPath,
            },
          });
        } catch (err) {
          if (err instanceof ApiError && err.status === 404) {
            await apiClient.createDocument({
              document: {
                title: status.prompt_path as string,
                body_markdown: body,
                path: status.prompt_path as DocumentPath,
              },
            });
          } else {
            throw err;
          }
        }
      }

      const nextStatuses = [...statuses, status];
      return apiClient.updateProject(projectId, {
        project: { ...projectRecord.project, statuses: nextStatuses },
      });
    },
    onMutate: async (status) => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const nextProject = {
          ...projectRecord.project,
          statuses: [...statuses, status],
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
        err instanceof Error ? err.message : "Failed to add status",
        "error",
      );
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({
        queryKey: ["project", response.project_id],
      });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
      queryClient.invalidateQueries({ queryKey: ["documentPaths"] });
      queryClient.invalidateQueries({ queryKey: ["documentsAtPath"] });
      addToast("Status added", "success");
      onClose();
    },
  });

  const canSave =
    !!key && !nameError && !!name.trim() && !saveMutation.isPending;

  const handleSave = useCallback(() => {
    if (!canSave) return;
    const status: StatusDefinition = {
      ...blankStatus(statuses.length),
      key,
      label: name.trim(),
      prompt_path: promptPath as DocumentPath,
    };
    saveMutation.mutate(status);
  }, [canSave, key, name, promptPath, statuses, saveMutation]);

  return (
    <div className={styles.body} data-testid="status-settings-form">
      <Input
        label="Name"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="Blocked"
        required
        data-testid="status-settings-name"
      />
      <Textarea
        label="Prompt"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder={`# Prompt for this status\n\nWrite the prompt the assignee sees on enter.`}
        rows={12}
        data-testid="status-settings-prompt-body"
      />
      {nameError && (
        <span className={styles.error} data-testid="status-settings-new-error">
          {nameError}
        </span>
      )}

      <div className={styles.actions} data-testid="status-settings-actions">
        <div className={styles.actionsLeft} />
        <div className={styles.actionsRight}>
          <Button
            variant="secondary"
            size="md"
            onClick={onClose}
            disabled={saveMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSave}
            disabled={!canSave}
            data-testid="status-settings-save"
          >
            {saveMutation.isPending ? "Adding…" : "Add status"}
          </Button>
        </div>
      </div>
    </div>
  );
}
