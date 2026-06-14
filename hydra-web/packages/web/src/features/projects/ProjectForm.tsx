import { useCallback, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Button,
  Icons,
  Input,
  Picker,
  PickerRow,
  Textarea,
} from "@hydra/ui";
import type {
  DocumentPath,
  ListProjectsResponse,
  Project,
  ProjectId,
  SessionSettings,
  Timeout,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useProjects } from "./useProjects";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticDelete,
  applyOptimisticUpsert,
} from "./projectCache";
import { slugifyStatusKey } from "./statusDefaults";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import { upsertPromptDoc, usePromptDocumentBody } from "./promptDocument";
import styles from "./ProjectForm.module.css";

export interface ProjectFormProps {
  creator: string;
  onClose: () => void;
  /** Present → edit mode (project settings); absent → create. */
  projectId?: ProjectId | null;
  /** Existing project, edit mode only. */
  initial?: Project;
  /** Active-issue count for the archive-confirmation hint (edit mode). */
  issueCount?: number;
}

// Shared Name + inline-prompt form behind both the New Project modal and the
// Project Settings modal, so the two stay identical. The key is derived from
// the name (never an editable field) and shown read-only alongside the prompt
// path; renaming re-keys the project end-to-end. Edit mode adds an Archive
// control and preserves the project's existing statuses (those are managed on
// the board, not here).
export function ProjectForm({
  creator,
  onClose,
  projectId,
  initial,
  issueCount,
}: ProjectFormProps) {
  const isEdit = !!projectId;
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: existingProjects } = useProjects();

  const [name, setName] = useState(initial?.name ?? "");
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [session, setSession] = useState<SessionSettings>(
    () => initial?.session_settings ?? {},
  );
  const [sessionSettingsOpen, setSessionSettingsOpen] = useState(false);
  const [idleTimeoutPickerOpen, setIdleTimeoutPickerOpen] = useState(false);

  const key = useMemo(() => slugifyStatusKey(name), [name]);
  const promptPath = key ? `/projects/${key}/prompt.md` : "";

  // Validate against the OTHER projects so renaming to the same key is fine.
  const reservedKeys = useMemo(
    () =>
      new Set(
        (existingProjects ?? [])
          .filter((p) => p.project_id !== projectId)
          .map((p) => p.project.key as string),
      ),
    [existingProjects, projectId],
  );

  const nameError = useMemo(() => {
    if (!name.trim()) return null;
    if (!key) return "Project name must include a letter or digit";
    if (reservedKeys.has(key)) return `Project '${key}' already exists`;
    return null;
  }, [name, key, reservedKeys]);

  // Seed the inline prompt editor from the project's existing document (edit
  // mode); a stable path so renaming doesn't refetch and discard edits.
  const initialPromptPath = useMemo(() => {
    if (!isEdit || !initial) return null;
    const existing = initial.prompt_path?.trim();
    return existing || (initial.key ? `/projects/${initial.key}/prompt.md` : null);
  }, [isEdit, initial]);
  const { body, setBody, loading: promptLoading } =
    usePromptDocumentBody(initialPromptPath);

  // Collapse the session_settings payload back to `undefined` only when every
  // subfield is empty — surfaced AND un-surfaced. CLI users can set
  // `repo_name` / `remote_url` / `branch` / `secrets` (not on the form), so
  // preserving them is required to round-trip the form without dropping
  // CLI-only overrides. Mirrors the StatusSettingsModal patchSession check.
  const collapsedSession = useMemo<SessionSettings | undefined>(() => {
    const allEmpty =
      (session.image ?? null) == null &&
      (session.model ?? null) == null &&
      (session.cpu_limit ?? null) == null &&
      (session.memory_limit ?? null) == null &&
      (session.max_retries ?? null) == null &&
      (session.idle_timeout ?? null) == null &&
      (session.repo_name ?? null) == null &&
      (session.remote_url ?? null) == null &&
      (session.branch ?? null) == null &&
      !(session.secrets && session.secrets.length > 0);
    return allEmpty ? undefined : session;
  }, [session]);

  const buildProject = useCallback(
    (): Project => ({
      ...(initial ?? {}),
      key: key as Project["key"],
      name: name.trim(),
      // Statuses are owned by the board's column controls, not this form.
      statuses: initial?.statuses ?? [],
      creator: (initial?.creator ?? creator) as Project["creator"],
      archived: false,
      prompt_path: (promptPath || null) as DocumentPath | null,
      priority: initial?.priority ?? 0,
      session_settings: collapsedSession,
    }),
    [initial, key, name, creator, promptPath, collapsedSession],
  );

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (body.trim() && promptPath) {
        // Edit re-points an existing doc (upsert); create writes a fresh one.
        if (isEdit) {
          await upsertPromptDoc(promptPath, body);
        } else {
          await apiClient.createDocument({
            document: {
              title: promptPath,
              body_markdown: body,
              path: promptPath as DocumentPath,
            },
          });
        }
      }
      const request = {
        key: key as Project["key"],
        name: name.trim(),
        prompt_path: (promptPath || null) as DocumentPath | null,
        priority: initial?.priority ?? 0,
        session_settings: collapsedSession,
      };
      if (isEdit && projectId) {
        return apiClient.updateProject(projectId, request);
      }
      return apiClient.createProject(request);
    },
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const next: ListProjectsResponse = {
          projects: applyOptimisticUpsert(
            previous.projects,
            projectId ?? null,
            buildProject(),
          ),
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
        err instanceof Error ? err.message : "Failed to save project",
        "error",
      );
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({
        queryKey: ["project", response.project_id],
      });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
      queryClient.invalidateQueries({ queryKey: ["documentByPath"] });
      queryClient.invalidateQueries({ queryKey: ["documentPaths"] });
      queryClient.invalidateQueries({ queryKey: ["documentsAtPath"] });
      addToast(isEdit ? "Project updated" : "Project created", "success");
      onClose();
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.archiveProject(projectId!),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
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
      addToast(
        err instanceof Error ? err.message : "Failed to archive project",
        "error",
      );
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
      addToast(
        issueCount && issueCount > 0
          ? `Project archived (${issueCount} issue(s) cascaded)`
          : "Project archived",
        "success",
      );
      setDeleteOpen(false);
      onClose();
    },
  });

  const canSave =
    !!key && !nameError && !!name.trim() && !saveMutation.isPending;

  const handleSave = useCallback(() => {
    if (!canSave) return;
    saveMutation.mutate();
  }, [canSave, saveMutation]);

  const setSessionString = (
    field: "image" | "model" | "cpu_limit" | "memory_limit",
    raw: string,
  ) => {
    setSession((prev) => ({ ...prev, [field]: raw === "" ? null : raw }));
  };

  const setSessionMaxRetries = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      setSession((prev) => ({ ...prev, max_retries: null }));
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n) || n < 0 || !Number.isInteger(n)) return;
    setSession((prev) => ({ ...prev, max_retries: n }));
  };

  // Idle timeout has three discrete modes: Default (None), Infinite, or an
  // explicit seconds count. The picker chooses the mode; the seconds input is
  // only meaningful in the "seconds" mode and converts on the fly.
  const idleTimeoutMode: "default" | "infinite" | "seconds" = (() => {
    const t = session.idle_timeout;
    if (t == null) return "default";
    if (t.kind === "infinite") return "infinite";
    return "seconds";
  })();
  const idleTimeoutSeconds =
    session.idle_timeout?.kind === "seconds"
      ? String(session.idle_timeout.value)
      : "";

  const setIdleTimeoutMode = (mode: "default" | "infinite" | "seconds") => {
    if (mode === "default") {
      setSession((prev) => ({ ...prev, idle_timeout: null }));
      return;
    }
    if (mode === "infinite") {
      setSession((prev) => ({ ...prev, idle_timeout: { kind: "infinite" } }));
      return;
    }
    const current =
      session.idle_timeout?.kind === "seconds"
        ? session.idle_timeout.value
        : (60n as unknown as bigint);
    setSession((prev) => ({
      ...prev,
      idle_timeout: { kind: "seconds", value: current } as Timeout,
    }));
  };

  const setIdleTimeoutSeconds = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      setSession((prev) => ({ ...prev, idle_timeout: null }));
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n) || n < 1 || !Number.isInteger(n)) return;
    setSession((prev) => ({
      ...prev,
      idle_timeout: { kind: "seconds", value: n as unknown as bigint } as Timeout,
    }));
  };

  return (
    <div className={styles.form} data-testid="project-form">
      <Input
        label="Name"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="Engineering"
        required
        data-testid="project-form-name"
      />
      <Textarea
        label="Prompt"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder={
          promptLoading
            ? "Loading prompt…"
            : `# Prompt for this project\n\nWrite the prompt agents on this project see.`
        }
        rows={12}
        disabled={promptLoading}
        data-testid="project-form-prompt-body"
      />
      <div className={styles.sessionSettings}>
        <button
          type="button"
          className={styles.collapsibleSummary}
          aria-expanded={sessionSettingsOpen}
          onClick={() => setSessionSettingsOpen((v) => !v)}
          data-testid="project-form-session-settings-toggle"
        >
          <span className={styles.collapsibleChevron} aria-hidden="true">
            {sessionSettingsOpen ? (
              <Icons.IconChevronDown size={10} />
            ) : (
              <Icons.IconChevronRight size={10} />
            )}
          </span>
          <span className={styles.sectionTitle}>Default session settings</span>
        </button>
        {sessionSettingsOpen && (
          <div
            className={styles.collapsibleContent}
            data-testid="project-form-session-settings-content"
          >
            <span className={styles.helpText}>
              Per-project defaults applied when spawning sessions for issues
              in this project. Issue- and status-level settings still win
              over these. Leave blank to inherit the global defaults.
            </span>
            <div className={styles.sessionInputs}>
              <Input
                label="CPU limit"
                value={session.cpu_limit ?? ""}
                onChange={(e) => setSessionString("cpu_limit", e.target.value)}
                placeholder="e.g. 500m, 2"
                data-testid="project-form-cpu-limit"
              />
              <Input
                label="Memory limit"
                value={session.memory_limit ?? ""}
                onChange={(e) =>
                  setSessionString("memory_limit", e.target.value)
                }
                placeholder="e.g. 1Gi, 512Mi"
                data-testid="project-form-memory-limit"
              />
            </div>
            <div className={styles.sessionInputs}>
              <Input
                label="Container image"
                value={session.image ?? ""}
                onChange={(e) => setSessionString("image", e.target.value)}
                placeholder="ghcr.io/org/image:tag"
                data-testid="project-form-image"
              />
              <Input
                label="Model"
                value={session.model ?? ""}
                onChange={(e) => setSessionString("model", e.target.value)}
                placeholder="e.g. claude-opus-4-7"
                data-testid="project-form-model"
              />
            </div>
            <div className={styles.sessionInputs}>
              <Input
                label="Max retries"
                type="number"
                min={0}
                step={1}
                value={
                  session.max_retries == null ? "" : String(session.max_retries)
                }
                onChange={(e) => setSessionMaxRetries(e.target.value)}
                placeholder="Inherit"
                data-testid="project-form-max-retries"
              />
              <span className={styles.spacer} />
            </div>
            <div
              className={styles.idleTimeout}
              data-testid="project-form-idle-timeout"
            >
              <label className={styles.label}>Idle timeout</label>
              <div className={styles.idleTimeoutInputs}>
                <Input
                  type="number"
                  min={1}
                  step={1}
                  value={idleTimeoutSeconds}
                  onChange={(e) => setIdleTimeoutSeconds(e.target.value)}
                  disabled={idleTimeoutMode !== "seconds"}
                  placeholder={
                    idleTimeoutMode === "infinite" ? "Never" : "Seconds"
                  }
                  aria-label="Idle timeout seconds"
                  data-testid="project-form-idle-timeout-seconds"
                />
                <Picker
                  label="Idle timeout"
                  hideLabel
                  open={idleTimeoutPickerOpen}
                  onToggle={() => setIdleTimeoutPickerOpen((v) => !v)}
                  value={
                    idleTimeoutMode === "default" ? (
                      <span className={styles.pillEmpty}>Server default</span>
                    ) : idleTimeoutMode === "infinite" ? (
                      <span>Never</span>
                    ) : (
                      <span>Custom</span>
                    )
                  }
                  data-testid="project-form-idle-timeout-mode"
                >
                  <PickerRow
                    active={idleTimeoutMode === "default"}
                    onClick={() => {
                      setIdleTimeoutMode("default");
                      setIdleTimeoutPickerOpen(false);
                    }}
                  >
                    <span>Server default</span>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                  <PickerRow
                    active={idleTimeoutMode === "seconds"}
                    onClick={() => {
                      setIdleTimeoutMode("seconds");
                      setIdleTimeoutPickerOpen(false);
                    }}
                  >
                    <span>Custom (seconds)</span>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                  <PickerRow
                    active={idleTimeoutMode === "infinite"}
                    onClick={() => {
                      setIdleTimeoutMode("infinite");
                      setIdleTimeoutPickerOpen(false);
                    }}
                  >
                    <span>Never</span>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                </Picker>
              </div>
            </div>
          </div>
        )}
      </div>
      <div className={styles.notes}>
        <span className={styles.readOnlyNote} data-testid="project-form-key">
          Key: {key || "<key>"}
        </span>
        <span
          className={styles.readOnlyNote}
          data-testid="project-form-prompt-path"
        >
          Prompt saved to {promptPath || "/projects/<key>/prompt.md"}
        </span>
      </div>
      {nameError && (
        <span className={styles.error} data-testid="project-form-error">
          {nameError}
        </span>
      )}
      <div className={styles.actions}>
        {isEdit && (
          <Button
            variant="danger-subtle"
            size="md"
            onClick={() => setDeleteOpen(true)}
            disabled={saveMutation.isPending || deleteMutation.isPending}
            data-testid="project-form-delete"
          >
            Archive project
          </Button>
        )}
        <span className={styles.actionsSpacer} />
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
          data-testid="project-form-save"
        >
          {saveMutation.isPending
            ? isEdit
              ? "Saving…"
              : "Creating…"
            : isEdit
            ? "Save"
            : "Create"}
        </Button>
      </div>
      {isEdit && (
        <DeleteConfirmModal
          open={deleteOpen}
          onClose={() => setDeleteOpen(false)}
          entityName={initial?.name ?? key}
          entityLabel="Project"
          actionLabel="Archive"
          pendingLabel="Archiving..."
          description={
            issueCount && issueCount > 0
              ? `${issueCount} issue(s) in this project will be archived.`
              : undefined
          }
          onConfirm={() => deleteMutation.mutate()}
          isPending={deleteMutation.isPending}
        />
      )}
    </div>
  );
}
