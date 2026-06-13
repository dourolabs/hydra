import { useCallback, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Textarea } from "@hydra/ui";
import type {
  DocumentPath,
  ListProjectsResponse,
  Project,
  ProjectId,
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
// path; renaming re-keys the project end-to-end. Edit mode adds a Delete
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
    }),
    [initial, key, name, creator, promptPath],
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
            Delete project
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
