import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type {
  DocumentPath,
  ListProjectsResponse,
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
import {
  principalKind,
  principalToPath,
  pathToPrincipal,
  type AssignKind,
} from "./principalAssign";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticDelete,
  applyOptimisticUpsert,
} from "./projectCache";
import { blankStatus } from "./statusDefaults";
import { PromptDocumentEditor } from "./PromptDocumentEditor";
import styles from "./ProjectEditor.module.css";

interface ProjectEditorProps {
  projectId?: ProjectId | null;
  initial?: Project;
  creator: string;
}

interface StatusRow {
  status: StatusDefinition;
  originalKey: string | null;
}

export function ProjectEditor({ projectId, initial, creator }: ProjectEditorProps) {
  const navigate = useNavigate();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  const isEdit = !!projectId;
  const [key, setKey] = useState(initial?.key ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [promptPath, setPromptPath] = useState(initial?.prompt_path ?? "");
  // Each row pairs the editor's current `StatusDefinition` with the
  // key it was loaded under. `originalKey === null` marks a row that
  // didn't exist on the server at mount (new project, or `+ Add
  // status`); existing rows preserve their load-time key so a Key
  // edit becomes a rename PUT instead of a delete-old + create-new
  // pair (which would 400 against the `ON DELETE RESTRICT` FK on
  // `issues_v2.status_sequence`).
  const [rows, setRows] = useState<StatusRow[]>(() =>
    initial?.statuses
      ? initial.statuses.map((s) => ({ status: s, originalKey: s.key }))
      : defaultNewStatuses().map((s) => ({ status: s, originalKey: null })),
  );
  const statuses = useMemo(() => rows.map((r) => r.status), [rows]);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [projectPromptExpanded, setProjectPromptExpanded] = useState(false);
  // Per-status expanded set keyed by index so each row's textarea stays
  // unmounted until its own toggle fires — avoids fanning out one
  // `getDocumentByPath` per status as soon as the editor opens.
  const [expandedStatuses, setExpandedStatuses] = useState<Set<number>>(
    () => new Set(),
  );
  const projectKeyForPaths = (key.trim() || "<key>");
  const projectPromptDefault = `/projects/${projectKeyForPaths}/prompt.md`;

  const formError = useMemo(
    () => validate(key, name, statuses, promptPath),
    [key, name, statuses, promptPath],
  );

  interface SubmitPayload {
    request: UpsertProjectRequest;
    project: Project;
    nextStatuses: StatusDefinition[];
    initialStatuses: StatusDefinition[];
    // Parallel to `nextStatuses`: the key each row was loaded under
    // (or `null` for rows added during this edit). Drives the per-row
    // create/update/delete decision so a Key edit on an existing row
    // becomes one rename PUT instead of a delete + create pair.
    originalKeys: (string | null)[];
  }

  const mutation = useMutation({
    mutationFn: async (payload: SubmitPayload) => {
      const upsertResp =
        isEdit && projectId
          ? await apiClient.updateProject(projectId, payload.request)
          : await apiClient.createProject(payload.request);
      // Status-level changes go through per-status endpoints, keyed
      // off each row's `originalKey`. New rows (`originalKey === null`)
      // POST; existing rows PUT against their original key — a body
      // whose `key` differs from the path segment is treated as an
      // in-place rename by the backend, preserving the
      // `(project_id, sequence)` storage identity.
      //
      // Known limitation: a swap-rename (A→B + B→A in one save) will
      // 400 on the first PUT due to a transient key collision. We
      // surface the backend error via the existing toast and do not
      // try to special-case it.
      const ref = upsertResp.project_id;
      const survivingOriginalKeys = new Set<string>();
      for (let i = 0; i < payload.nextStatuses.length; i++) {
        const status = payload.nextStatuses[i];
        const originalKey = payload.originalKeys[i];
        if (originalKey === null) {
          await apiClient.createProjectStatus(ref, status);
        } else {
          await apiClient.updateProjectStatus(ref, originalKey, status);
          survivingOriginalKeys.add(originalKey);
        }
      }
      for (const initial of payload.initialStatuses) {
        if (!survivingOriginalKeys.has(initial.key)) {
          await apiClient.deleteProjectStatus(ref, initial.key);
        }
      }
      return upsertResp;
    },
    onMutate: async (payload) => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous = queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const next: ListProjectsResponse = {
          projects: applyOptimisticUpsert(previous.projects, projectId ?? null, payload.project),
        };
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, next);
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(PROJECTS_QUERY_KEY, context.previous);
      }
      addToast(err instanceof Error ? err.message : "Failed to save project", "error");
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
      addToast(isEdit ? "Project updated" : "Project created", "success");
      navigate(`/projects/${key.trim()}`);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.deleteProject(projectId!),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous = queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous && projectId) {
        const next: ListProjectsResponse = {
          projects: applyOptimisticDelete(previous.projects, projectId),
        };
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, next);
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(PROJECTS_QUERY_KEY, context.previous);
      }
      addToast(err instanceof Error ? err.message : "Failed to delete project", "error");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      addToast("Project deleted", "success");
      navigate("/projects");
    },
  });

  const handleSubmit = useCallback(() => {
    if (formError) {
      addToast(formError, "error");
      return;
    }
    const trimmedPromptPath = promptPath.trim();
    const promptPathOrNull = trimmedPromptPath ? trimmedPromptPath : null;
    const priority = initial?.priority ?? 0;
    const nextStatuses = rows.map((r) => normalizeStatusForSubmit(r.status));
    const originalKeys = rows.map((r) => r.originalKey);
    const project: Project = {
      key: key.trim(),
      name: name.trim(),
      statuses: nextStatuses,
      creator,
      deleted: false,
      prompt_path: promptPathOrNull,
      // Preserve the existing project's priority on edit; new projects
      // get `0` (sort-to-top).
      priority,
    };
    const request: UpsertProjectRequest = {
      key: key.trim(),
      name: name.trim(),
      prompt_path: promptPathOrNull,
      priority,
    };
    mutation.mutate({
      request,
      project,
      nextStatuses,
      initialStatuses: initial?.statuses ?? [],
      originalKeys,
    });
  }, [
    formError,
    key,
    name,
    promptPath,
    rows,
    creator,
    initial,
    mutation,
    addToast,
  ]);

  const updateStatus = useCallback(
    (index: number, patch: Partial<StatusDefinition>) => {
      setRows((prev) =>
        prev.map((r, i) =>
          i === index ? { ...r, status: { ...r.status, ...patch } } : r,
        ),
      );
    },
    [],
  );

  const removeStatus = useCallback((index: number) => {
    setRows((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const moveStatus = useCallback((index: number, delta: number) => {
    setRows((prev) => {
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
    setRows((prev) => [
      ...prev,
      { status: blankStatus(prev.length), originalKey: null },
    ]);
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
        <PromptDocumentEditor
          path={promptPath || null}
          defaultPath={projectPromptDefault}
          onPathChange={setPromptPath}
          expanded={projectPromptExpanded}
          onToggleExpanded={() => setProjectPromptExpanded((v) => !v)}
          label="Prompt path"
          placeholder="/projects/<key>/prompt.md"
          testId="project-editor-prompt-path"
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
              projectKey={projectKeyForPaths}
              promptExpanded={expandedStatuses.has(index)}
              onTogglePromptExpanded={() =>
                setExpandedStatuses((prev) => {
                  const next = new Set(prev);
                  if (next.has(index)) next.delete(index);
                  else next.add(index);
                  return next;
                })
              }
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
  projectKey: string;
  promptExpanded: boolean;
  onTogglePromptExpanded: () => void;
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
  projectKey,
  promptExpanded,
  onTogglePromptExpanded,
}: StatusEditorProps) {
  const onEnter = status.on_enter ?? null;
  const assignKind = principalKind(onEnter?.assign_to ?? null);
  const principalPath = onEnter?.assign_to ? principalToPath(onEnter.assign_to) : "";
  const external = onEnter?.assign_to && "External" in onEnter.assign_to
    ? onEnter.assign_to.External
    : null;
  const attachForm = onEnter?.attach_form ?? "";

  const userOptions: SelectOption[] = [
    { value: "", label: "— select user —" },
    ...users.map((u) => ({ value: `users/${u}`, label: u })),
  ];
  const agentOptions: SelectOption[] = [
    { value: "", label: "— select agent —" },
    ...agents.map((a) => ({ value: `agents/${a}`, label: a })),
  ];
  const kindOptions: SelectOption[] = [
    { value: "none", label: "— none —" },
    { value: "user", label: "User" },
    { value: "agent", label: "Agent" },
    { value: "external", label: "External" },
  ];

  const setAssign = (next: Principal | null) => {
    const nextForm = onEnter?.attach_form ?? null;
    if (!next && !nextForm) {
      onChange({ on_enter: null });
      return;
    }
    onChange({ on_enter: { assign_to: next, attach_form: nextForm } });
  };

  const setAttachForm = (raw: string) => {
    const nextForm = raw ? (raw as DocumentPath) : null;
    const nextAssign = onEnter?.assign_to ?? null;
    if (!nextAssign && !nextForm) {
      onChange({ on_enter: null });
      return;
    }
    onChange({ on_enter: { assign_to: nextAssign, attach_form: nextForm } });
  };

  const setKind = (kind: AssignKind) => {
    if (kind === "none") {
      setAssign(null);
      return;
    }
    if (kind === "user") {
      setAssign({ User: { name: users[0] ?? "" } });
      return;
    }
    if (kind === "agent") {
      setAssign({ Agent: { name: agents[0] ?? "" } });
      return;
    }
    setAssign({ External: { system: external?.system ?? "", username: external?.username ?? "" } });
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
        <label>
          <input
            type="checkbox"
            checked={status.interactive ?? false}
            onChange={(e) => onChange({ interactive: e.target.checked })}
            data-testid={`status-editor-interactive-${index}`}
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

      <PromptDocumentEditor
        path={status.prompt_path ?? null}
        defaultPath={`/projects/${projectKey}/statuses/${
          status.key.trim() || `status-${index + 1}`
        }.md`}
        onPathChange={(value) => onChange({ prompt_path: value })}
        expanded={promptExpanded}
        onToggleExpanded={onTogglePromptExpanded}
        label="Prompt path"
        placeholder="/projects/<key>/statuses/<status-key>.md"
        testId={`status-editor-prompt-path-${index}`}
      />
    </div>
  );
}

function validate(
  key: string,
  name: string,
  statuses: StatusDefinition[],
  promptPath: string,
): string | null {
  if (!key.trim()) return "Project key is required";
  if (!/^[a-z0-9-]+$/.test(key.trim())) {
    return "Project key must be lowercase letters, digits, and dashes only";
  }
  if (!name.trim()) return "Project name is required";
  if (statuses.length === 0) return "At least one status is required";

  if (!isPromptPathValid(promptPath)) {
    return "Project prompt path must be a doc-store path starting with '/'";
  }

  const seen = new Set<string>();
  for (const s of statuses) {
    if (!s.key.trim()) return "Every status needs a key";
    if (!/^[a-z0-9-]+$/.test(s.key)) {
      return `Status key '${s.key}' must be lowercase letters, digits, and dashes only`;
    }
    if (seen.has(s.key)) return `Duplicate status key '${s.key}'`;
    seen.add(s.key);
    if (!isPromptPathValid(s.prompt_path ?? "")) {
      return `Status '${s.key}' prompt path must be a doc-store path starting with '/'`;
    }
  }
  return null;
}

// Light client-side check — empty values clear the field; non-empty
// values must look like a doc-store path. The server is authoritative.
function isPromptPathValid(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return true;
  return trimmed.startsWith("/");
}

// Convert the editor's `prompt_path: ""` placeholder back to `null`
// before sending the project to the server.
function normalizeStatusForSubmit(status: StatusDefinition): StatusDefinition {
  const trimmed = status.prompt_path?.trim() ?? "";
  return { ...status, prompt_path: trimmed ? trimmed : null };
}

function defaultNewStatuses(): StatusDefinition[] {
  return [
    {
      key: "open",
      label: "Open",
      color: LABEL_COLOR_PALETTE[5],
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      on_enter: null,
      prompt_path: null,
      position: 100,
    },
    {
      key: "in-progress",
      label: "In progress",
      color: LABEL_COLOR_PALETTE[2],
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      on_enter: null,
      prompt_path: null,
      position: 200,
    },
    {
      key: "closed",
      label: "Closed",
      color: LABEL_COLOR_PALETTE[3],
      unblocks_parents: true,
      unblocks_dependents: true,
      cascades_to_children: false,
      on_enter: null,
      prompt_path: null,
      position: 300,
    },
  ];
}
