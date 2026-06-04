import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type {
  DocumentPath,
  Principal,
  Project,
  ProjectId,
  StatusDefinition,
  UpsertProjectRequest,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useAgents } from "../../hooks/useAgents";
import { useUsers } from "../../hooks/useUsers";
import { ColorPicker, LABEL_COLOR_PALETTE } from "../../components/ColorPicker";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import styles from "./ProjectEditor.module.css";

interface ProjectEditorProps {
  /** Existing project to edit, or null/undefined to create a new one. */
  projectId?: ProjectId | null;
  initial?: Project;
  creator: string;
}

/**
 * Create / edit a project, including its status list and optional
 * `on_enter` automation per status. Reuses the shared `ColorPicker` (and
 * its `LABEL_COLOR_PALETTE`) so label and status creation share UI per
 * `/designs/per-project-issue-statuses.md` §4 "Frontend display".
 */
export function ProjectEditor({ projectId, initial, creator }: ProjectEditorProps) {
  const navigate = useNavigate();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  const isEdit = !!projectId;
  const [key, setKey] = useState(initial?.key ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [statuses, setStatuses] = useState<StatusDefinition[]>(
    initial?.statuses ?? defaultNewStatuses(),
  );
  const [defaultStatusKey, setDefaultStatusKey] = useState<string>(
    initial?.default_status_key ?? statuses[0]?.key ?? "",
  );
  const [deleteOpen, setDeleteOpen] = useState(false);

  const defaultStatusOptions: SelectOption[] = useMemo(
    () => statuses.map((s) => ({ value: s.key, label: s.label || s.key })),
    [statuses],
  );

  const formError = useMemo(() => validate(key, name, statuses, defaultStatusKey), [
    key,
    name,
    statuses,
    defaultStatusKey,
  ]);

  const mutation = useMutation({
    mutationFn: async (req: UpsertProjectRequest) => {
      if (isEdit && projectId) {
        return apiClient.updateProject(projectId, req);
      }
      return apiClient.createProject(req);
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
      addToast(isEdit ? "Project updated" : "Project created", "success");
      navigate(`/projects/${key.trim()}`);
    },
    onError: (err) => {
      addToast(err instanceof Error ? err.message : "Failed to save project", "error");
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.deleteProject(projectId!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
      addToast("Project deleted", "success");
      navigate("/projects");
    },
    onError: (err) => {
      addToast(err instanceof Error ? err.message : "Failed to delete project", "error");
    },
  });

  const handleSubmit = useCallback(() => {
    if (formError) {
      addToast(formError, "error");
      return;
    }
    const project: Project = {
      key: key.trim(),
      name: name.trim(),
      statuses,
      default_status_key: defaultStatusKey,
      creator,
      deleted: false,
    };
    mutation.mutate({ project });
  }, [formError, key, name, statuses, defaultStatusKey, creator, mutation, addToast]);

  const updateStatus = useCallback(
    (index: number, patch: Partial<StatusDefinition>) => {
      setStatuses((prev) =>
        prev.map((s, i) => (i === index ? { ...s, ...patch } : s)),
      );
    },
    [],
  );

  const removeStatus = useCallback(
    (index: number) => {
      setStatuses((prev) => {
        const next = prev.filter((_, i) => i !== index);
        if (defaultStatusKey && !next.some((s) => s.key === defaultStatusKey) && next[0]) {
          setDefaultStatusKey(next[0].key);
        }
        return next;
      });
    },
    [defaultStatusKey],
  );

  const moveStatus = useCallback((index: number, delta: number) => {
    setStatuses((prev) => {
      const target = index + delta;
      if (target < 0 || target >= prev.length) return prev;
      const next = [...prev];
      const tmp = next[index];
      next[index] = next[target];
      next[target] = tmp;
      return next;
    });
  }, []);

  const addStatus = useCallback(() => {
    setStatuses((prev) => [...prev, blankStatus(prev.length)]);
  }, []);

  return (
    <div className={styles.editor} data-testid="project-editor">
      <div className={styles.row}>
        <label className={styles.label}>Project key</label>
        <Input
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="engineering"
          disabled={isEdit}
          required
          data-testid="project-editor-key"
        />
      </div>
      <div className={styles.row}>
        <label className={styles.label}>Name</label>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Engineering"
          required
          data-testid="project-editor-name"
        />
      </div>
      <div className={styles.row}>
        <label className={styles.label}>Default status</label>
        <Select
          options={defaultStatusOptions}
          value={defaultStatusKey}
          onChange={(e) => setDefaultStatusKey(e.target.value)}
          data-testid="project-editor-default-status"
        />
      </div>

      <div className={styles.row}>
        <label className={styles.label}>Statuses</label>
        <div className={styles.statusList}>
          {statuses.map((status, index) => (
            <StatusEditor
              key={`status-${index}`}
              status={status}
              index={index}
              count={statuses.length}
              onChange={(patch) => updateStatus(index, patch)}
              onRemove={() => removeStatus(index)}
              onMove={(delta) => moveStatus(index, delta)}
              agents={agents?.map((a) => a.name) ?? []}
              users={users?.map((u) => u.username) ?? []}
            />
          ))}
        </div>
        <button
          type="button"
          className={styles.miniButton}
          onClick={addStatus}
          data-testid="project-editor-add-status"
        >
          + Add status
        </button>
      </div>

      {formError && <span className={styles.error}>{formError}</span>}

      <div className={styles.actions}>
        {isEdit && (
          <Button variant="ghost" size="md" onClick={() => setDeleteOpen(true)}>
            Delete project
          </Button>
        )}
        <Button variant="secondary" size="md" onClick={() => navigate("/projects")}>
          Cancel
        </Button>
        <Button
          variant="primary"
          size="md"
          onClick={handleSubmit}
          disabled={!!formError || mutation.isPending}
          data-testid="project-editor-save"
        >
          {mutation.isPending ? "Saving…" : isEdit ? "Save" : "Create"}
        </Button>
      </div>

      {isEdit && (
        <DeleteConfirmModal
          open={deleteOpen}
          onClose={() => setDeleteOpen(false)}
          entityName={initial?.name ?? key}
          entityLabel="Project"
          onConfirm={() => deleteMutation.mutate()}
          isPending={deleteMutation.isPending}
        />
      )}
    </div>
  );
}

interface StatusEditorProps {
  status: StatusDefinition;
  index: number;
  count: number;
  onChange: (patch: Partial<StatusDefinition>) => void;
  onRemove: () => void;
  onMove: (delta: number) => void;
  agents: string[];
  users: string[];
}

function StatusEditor({
  status,
  index,
  count,
  onChange,
  onRemove,
  onMove,
  agents,
  users,
}: StatusEditorProps) {
  const onEnter = status.on_enter ?? null;
  const assignPath = onEnter?.assign_to ? principalToPath(onEnter.assign_to) : "";
  const attachForm = onEnter?.attach_form ?? "";

  const assigneeOptions: SelectOption[] = [
    { value: "", label: "— none —" },
    ...agents.map((a) => ({ value: `agents/${a}`, label: `Agent · ${a}` })),
    ...users.map((u) => ({ value: `users/${u}`, label: `User · ${u}` })),
  ];

  const updateOnEnter = (patch: { assignPath?: string; attachForm?: string }) => {
    const nextAssign =
      patch.assignPath !== undefined
        ? pathToPrincipal(patch.assignPath)
        : onEnter?.assign_to ?? null;
    const nextForm =
      patch.attachForm !== undefined
        ? patch.attachForm
          ? (patch.attachForm as DocumentPath)
          : null
        : onEnter?.attach_form ?? null;
    if (!nextAssign && !nextForm) {
      onChange({ on_enter: null });
      return;
    }
    onChange({
      on_enter: {
        assign_to: nextAssign,
        attach_form: nextForm,
      },
    });
  };

  return (
    <div className={styles.statusCard} data-testid={`status-editor-${index}`}>
      <div className={styles.statusHeader}>
        <button
          type="button"
          className={styles.miniButton}
          onClick={() => onMove(-1)}
          disabled={index === 0}
          aria-label="Move up"
        >
          ↑
        </button>
        <button
          type="button"
          className={styles.miniButton}
          onClick={() => onMove(1)}
          disabled={index === count - 1}
          aria-label="Move down"
        >
          ↓
        </button>
        <span className={styles.label}>Status #{index + 1}</span>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          className={`${styles.miniButton} ${styles.miniButtonDanger}`}
          onClick={onRemove}
          disabled={count <= 1}
          data-testid={`status-editor-remove-${index}`}
        >
          Remove
        </button>
      </div>
      <div className={styles.statusInputs}>
        <Input
          label="Key"
          value={status.key}
          onChange={(e) => onChange({ key: e.target.value })}
          placeholder="in-progress"
          required
          data-testid={`status-editor-key-${index}`}
        />
        <Input
          label="Label"
          value={status.label}
          onChange={(e) => onChange({ label: e.target.value })}
          placeholder="In progress"
          required
        />
        <Input
          label="Icon"
          value={status.icon}
          onChange={(e) => onChange({ icon: e.target.value })}
          placeholder="circle"
        />
      </div>

      <div className={styles.row}>
        <label className={styles.label}>Color</label>
        <ColorPicker
          value={status.color}
          onChange={(color) => onChange({ color })}
          palette={LABEL_COLOR_PALETTE}
          allowCustom
        />
      </div>

      <div className={styles.flagRow}>
        <label>
          <input
            type="checkbox"
            checked={status.unblocks_parents}
            onChange={(e) => onChange({ unblocks_parents: e.target.checked })}
          />
          Unblocks parents (terminal)
        </label>
        <label>
          <input
            type="checkbox"
            checked={status.unblocks_dependents}
            onChange={(e) => onChange({ unblocks_dependents: e.target.checked })}
          />
          Unblocks dependents
        </label>
        <label>
          <input
            type="checkbox"
            checked={status.cascades_to_children}
            onChange={(e) => onChange({ cascades_to_children: e.target.checked })}
          />
          Cascades to children
        </label>
      </div>

      <div className={styles.onEnter}>
        <span className={styles.onEnterTitle}>On enter</span>
        <Select
          label="Assign to"
          options={assigneeOptions}
          value={assignPath}
          onChange={(e) => updateOnEnter({ assignPath: e.target.value })}
        />
        <Input
          label="Attach form"
          value={attachForm}
          onChange={(e) => updateOnEnter({ attachForm: e.target.value })}
          placeholder="/forms/review.yaml"
        />
      </div>
    </div>
  );
}

function validate(
  key: string,
  name: string,
  statuses: StatusDefinition[],
  defaultStatusKey: string,
): string | null {
  if (!key.trim()) return "Project key is required";
  if (!/^[a-z0-9-]+$/.test(key.trim())) {
    return "Project key must be lowercase letters, digits, and dashes only";
  }
  if (!name.trim()) return "Project name is required";
  if (statuses.length === 0) return "At least one status is required";

  const seen = new Set<string>();
  for (const s of statuses) {
    if (!s.key.trim()) return "Every status needs a key";
    if (!/^[a-z0-9-]+$/.test(s.key)) {
      return `Status key '${s.key}' must be lowercase letters, digits, and dashes only`;
    }
    if (seen.has(s.key)) return `Duplicate status key '${s.key}'`;
    seen.add(s.key);
    if (!s.label.trim()) return `Status '${s.key}' needs a label`;
  }
  if (!seen.has(defaultStatusKey)) {
    return `Default status '${defaultStatusKey}' must reference a declared status`;
  }
  return null;
}

function blankStatus(index: number): StatusDefinition {
  return {
    key: `status-${index + 1}`,
    label: "",
    icon: "circle",
    color: LABEL_COLOR_PALETTE[index % LABEL_COLOR_PALETTE.length],
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    on_enter: null,
  };
}

function defaultNewStatuses(): StatusDefinition[] {
  return [
    {
      key: "open",
      label: "Open",
      icon: "circle",
      color: LABEL_COLOR_PALETTE[5],
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      on_enter: null,
    },
    {
      key: "in-progress",
      label: "In progress",
      icon: "circle-half",
      color: LABEL_COLOR_PALETTE[2],
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      on_enter: null,
    },
    {
      key: "closed",
      label: "Closed",
      icon: "check",
      color: LABEL_COLOR_PALETTE[3],
      unblocks_parents: true,
      unblocks_dependents: true,
      cascades_to_children: false,
      on_enter: null,
    },
  ];
}

function principalToPath(p: Principal): string {
  if ("Agent" in p) return `agents/${p.Agent.name}`;
  if ("User" in p) return `users/${p.User.name}`;
  return "";
}

function pathToPrincipal(path: string): Principal | null {
  if (!path) return null;
  if (path.startsWith("agents/")) return { Agent: { name: path.slice(7) } };
  if (path.startsWith("users/")) return { User: { name: path.slice(6) } };
  return null;
}
