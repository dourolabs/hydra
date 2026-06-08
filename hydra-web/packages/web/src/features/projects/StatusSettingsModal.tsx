import { useCallback, useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Modal, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type {
  DocumentPath,
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
import styles from "./StatusSettingsModal.module.css";

export interface StatusSettingsModalProps {
  open: boolean;
  onClose: () => void;
  projectRecord: ProjectRecord;
  statusKey: string;
  issueCount: number;
}

export function StatusSettingsModal({
  open,
  onClose,
  projectRecord,
  statusKey,
  issueCount,
}: StatusSettingsModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  const statuses = projectRecord.project.statuses;
  const index = statuses.findIndex((s) => s.key === statusKey);
  const initialStatus = index >= 0 ? statuses[index] : null;

  const [draft, setDraft] = useState<StatusDefinition | null>(initialStatus);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  // Resync local draft whenever the modal is opened against a different
  // status (gear click on another column reuses the same component instance).
  useEffect(() => {
    setDraft(initialStatus);
    setConfirmingDelete(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, projectRecord.project_id, statusKey, projectRecord.version]);

  const saveMutation = useMutation({
    mutationFn: async (nextStatuses: StatusDefinition[]) => {
      return apiClient.updateProject(projectRecord.project_id, {
        project: { ...projectRecord.project, statuses: nextStatuses },
      });
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
      addToast("Status updated", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update status",
        "error",
      );
    },
  });

  const onlyStatus = statuses.length <= 1;
  const hasIssues = issueCount > 0;
  const canDelete = !onlyStatus && !hasIssues;
  const deleteTooltip = onlyStatus
    ? "Cannot delete the only status"
    : hasIssues
    ? `Cannot delete a status with ${issueCount} open issues; move them first`
    : "";

  const handleSave = useCallback(() => {
    if (!draft || index < 0) return;
    const trimmedPromptPath = draft.prompt_path?.trim() ?? "";
    const normalized: StatusDefinition = {
      ...draft,
      prompt_path: trimmedPromptPath ? trimmedPromptPath : null,
    };
    const next = statuses.map((s, i) => (i === index ? normalized : s));
    saveMutation.mutate(next);
  }, [draft, index, statuses, saveMutation]);

  const handleMove = useCallback(
    (delta: number) => {
      if (index < 0) return;
      const target = index + delta;
      if (target < 0 || target >= statuses.length) return;
      const next = [...statuses];
      const tmp = next[index];
      next[index] = next[target];
      next[target] = tmp;
      saveMutation.mutate(next);
    },
    [index, statuses, saveMutation],
  );

  const handleDelete = useCallback(() => {
    if (!canDelete || index < 0) return;
    const next = statuses.filter((_, i) => i !== index);
    saveMutation.mutate(next);
  }, [canDelete, index, statuses, saveMutation]);

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

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Status — ${draft.label || draft.key}`}
    >
      <StatusForm
        draft={draft}
        setDraft={setDraft}
        agents={agents?.map((a) => a.name) ?? []}
        users={users?.map((u) => u.username) ?? []}
        index={index}
        count={statuses.length}
        onMove={handleMove}
        saving={saveMutation.isPending}
      />

      <div className={styles.actions} data-testid="status-settings-actions">
        <div className={styles.actionsLeft}>
          {confirmingDelete ? (
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
            disabled={saveMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSave}
            disabled={saveMutation.isPending}
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
  index: number;
  count: number;
  onMove: (delta: number) => void;
  saving: boolean;
}

function StatusForm({
  draft,
  setDraft,
  agents,
  users,
  index,
  count,
  onMove,
  saving,
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
  const kindOptions: SelectOption[] = [
    { value: "none", label: "— none —" },
    { value: "user", label: "User" },
    { value: "agent", label: "Agent" },
    { value: "external", label: "External" },
  ];

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
    if (kind === "user") return setAssign({ User: { name: users[0] ?? "" } });
    if (kind === "agent") return setAssign({ Agent: { name: agents[0] ?? "" } });
    setAssign({
      External: { system: external?.system ?? "", username: external?.username ?? "" },
    });
  };

  return (
    <div className={styles.body} data-testid="status-settings-form">
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
