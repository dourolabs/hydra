import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Modal, Textarea } from "@hydra/ui";
import type {
  DocumentPath,
  ListProjectsResponse,
  Project,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useUsername } from "../auth/useUsername";
import { useProjects } from "./useProjects";
import { PROJECTS_QUERY_KEY, applyOptimisticUpsert } from "./projectCache";
import { slugifyStatusKey } from "./statusDefaults";
import styles from "./ProjectCreateModal.module.css";

interface ProjectCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function ProjectCreateModal({ open, onClose }: ProjectCreateModalProps) {
  // Rendered only under `AppLayout`'s auth guard, so the user is non-null here.
  const username = useUsername();
  if (!username) return null;
  return (
    <Modal open={open} onClose={onClose} title="New project">
      <NewProjectForm creator={username} onClose={onClose} />
    </Modal>
  );
}

interface NewProjectFormProps {
  creator: string;
  onClose: () => void;
}

function NewProjectForm({ creator, onClose }: NewProjectFormProps) {
  const navigate = useNavigate();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { data: existingProjects } = useProjects();

  const [name, setName] = useState("");
  const [body, setBody] = useState("");

  const key = useMemo(() => slugifyStatusKey(name), [name]);
  const existingKeys = useMemo(
    () => new Set((existingProjects ?? []).map((p) => p.project.key as string)),
    [existingProjects],
  );
  const promptPath = key ? `/projects/${key}/prompt.md` : "";

  const nameError = useMemo(() => {
    if (!name.trim()) return null;
    if (!key) return "Project name must include a letter or digit";
    if (existingKeys.has(key)) {
      return `Project '${key}' already exists`;
    }
    return null;
  }, [name, key, existingKeys]);

  const saveMutation = useMutation({
    mutationFn: async () => {
      const project: Project = {
        key: key as Project["key"],
        name: name.trim(),
        statuses: [],
        creator: creator as Project["creator"],
        deleted: false,
        prompt_path: promptPath,
        priority: 0,
      };
      const trimmedBody = body.trim();
      if (trimmedBody) {
        await apiClient.createDocument({
          document: {
            title: promptPath,
            body_markdown: body,
            path: promptPath as DocumentPath,
          },
        });
      }
      return apiClient.createProject({ project });
    },
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const project: Project = {
          key: key as Project["key"],
          name: name.trim(),
          statuses: [],
          creator: creator as Project["creator"],
          deleted: false,
          prompt_path: promptPath,
          priority: 0,
        };
        const next: ListProjectsResponse = {
          projects: applyOptimisticUpsert(previous.projects, null, project),
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
        err instanceof Error ? err.message : "Failed to create project",
        "error",
      );
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({
        queryKey: ["project", response.project_id],
      });
      queryClient.invalidateQueries({ queryKey: ["documentPaths"] });
      queryClient.invalidateQueries({ queryKey: ["documentsAtPath"] });
      addToast("Project created", "success");
      onClose();
      navigate(`/projects/${key}`);
    },
  });

  const canSave =
    !!key && !nameError && !!name.trim() && !saveMutation.isPending;

  const handleSave = useCallback(() => {
    if (!canSave) return;
    saveMutation.mutate();
  }, [canSave, saveMutation]);

  return (
    <div className={styles.form} data-testid="new-project-form">
      <Input
        label="Name"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="Engineering"
        required
        data-testid="new-project-name"
      />
      <Textarea
        label="Prompt"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder={`# Prompt for this project\n\nWrite the prompt agents on this project see.`}
        rows={12}
        data-testid="new-project-prompt-body"
      />
      {nameError && (
        <span className={styles.error} data-testid="new-project-error">
          {nameError}
        </span>
      )}
      <div className={styles.actions}>
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
          data-testid="new-project-save"
        >
          {saveMutation.isPending ? "Creating…" : "Create"}
        </Button>
      </div>
    </div>
  );
}
