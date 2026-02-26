import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Button, Modal, Input } from "@metis/ui";
import type {
  RepositoryRecord,
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
} from "@metis/api";
import { apiClient } from "../api/client";
import { useRepositories } from "../hooks/useRepositories";
import { useToast } from "../features/toast/useToast";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const { data: repositories, isLoading, error } = useRepositories();
  const [createOpen, setCreateOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<RepositoryRecord | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RepositoryRecord | null>(null);

  return (
    <div className={styles.page}>
      <div className={styles.pageHeader}>
        <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
          Add Repository
        </Button>
      </div>

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>
          Failed to load repositories: {(error as Error).message}
        </p>
      )}

      {repositories && repositories.length === 0 && (
        <p className={styles.empty}>No repositories configured.</p>
      )}

      {repositories && repositories.length > 0 && (
        <Panel
          header={
            <span className={styles.sectionTitle}>Repositories</span>
          }
        >
          <table className={styles.repoTable}>
            <thead>
              <tr>
                <th>Name</th>
                <th>Remote URL</th>
                <th>Default Branch</th>
                <th>Default Image</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {repositories.map((repo) => (
                <RepositoryRow
                  key={repo.name}
                  repo={repo}
                  onEdit={() => setEditTarget(repo)}
                  onDelete={() => setDeleteTarget(repo)}
                />
              ))}
            </tbody>
          </table>
        </Panel>
      )}

      <RepositoryCreateModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
      />

      {editTarget && (
        <RepositoryEditModal
          open={!!editTarget}
          repo={editTarget}
          onClose={() => setEditTarget(null)}
        />
      )}

      {deleteTarget && (
        <RepositoryDeleteModal
          open={!!deleteTarget}
          repo={deleteTarget}
          onClose={() => setDeleteTarget(null)}
        />
      )}
    </div>
  );
}

interface RepositoryRowProps {
  repo: RepositoryRecord;
  onEdit: () => void;
  onDelete: () => void;
}

function RepositoryRow({ repo, onEdit, onDelete }: RepositoryRowProps) {
  return (
    <tr>
      <td>
        <span className={styles.repoName}>{repo.name}</span>
      </td>
      <td>
        <span className={styles.repoUrl}>{repo.repository.remote_url}</span>
      </td>
      <td>
        {repo.repository.default_branch ? (
          <span className={styles.repoBranch}>
            {repo.repository.default_branch}
          </span>
        ) : (
          <span className={styles.dimText}>—</span>
        )}
      </td>
      <td>
        {repo.repository.default_image ? (
          <span className={styles.repoImage}>
            {repo.repository.default_image}
          </span>
        ) : (
          <span className={styles.dimText}>—</span>
        )}
      </td>
      <td>
        <div className={styles.rowActions}>
          <Button variant="ghost" size="sm" onClick={onEdit}>
            Edit
          </Button>
          <Button variant="ghost" size="sm" onClick={onDelete}>
            Delete
          </Button>
        </div>
      </td>
    </tr>
  );
}

// ---------------------------------------------------------------------------
// Create Repository Modal
// ---------------------------------------------------------------------------

interface RepositoryCreateModalProps {
  open: boolean;
  onClose: () => void;
}

function RepositoryCreateModal({ open, onClose }: RepositoryCreateModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [name, setName] = useState("");
  const [remoteUrl, setRemoteUrl] = useState("");
  const [defaultBranch, setDefaultBranch] = useState("");
  const [defaultImage, setDefaultImage] = useState("");

  const resetForm = useCallback(() => {
    setName("");
    setRemoteUrl("");
    setDefaultBranch("");
    setDefaultImage("");
  }, []);

  const mutation = useMutation({
    mutationFn: (params: CreateRepositoryRequest) =>
      apiClient.createRepository(params),
    onSuccess: () => {
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      addToast("Repository created", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to create repository",
        "error",
      );
    },
  });

  const namePattern = /^[^/]+\/[^/]+$/;
  const nameValid = name.trim().length === 0 || namePattern.test(name.trim());
  const isValid =
    name.trim().length > 0 &&
    namePattern.test(name.trim()) &&
    remoteUrl.trim().length > 0;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      name: name.trim(),
      remote_url: remoteUrl.trim(),
      default_branch: defaultBranch.trim() || null,
      default_image: defaultImage.trim() || null,
    });
  }, [name, remoteUrl, defaultBranch, defaultImage, isValid, mutation]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      resetForm();
      onClose();
    }
  }, [mutation.isPending, resetForm, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="Add Repository">
      <div className={styles.formFields} onKeyDown={handleKeyDown}>
        <Input
          label="Name"
          placeholder="org/repo"
          value={name}
          onChange={(e) => setName(e.target.value)}
          error={
            !nameValid ? "Name must be in org/repo format" : undefined
          }
          required
        />
        <Input
          label="Remote URL"
          placeholder="https://github.com/org/repo.git"
          value={remoteUrl}
          onChange={(e) => setRemoteUrl(e.target.value)}
          required
        />
        <Input
          label="Default Branch"
          placeholder="main"
          value={defaultBranch}
          onChange={(e) => setDefaultBranch(e.target.value)}
        />
        <Input
          label="Default Image"
          placeholder="ghcr.io/org/repo:latest"
          value={defaultImage}
          onChange={(e) => setDefaultImage(e.target.value)}
        />
        <div className={styles.formActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={handleClose}
            disabled={mutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSubmit}
            disabled={!isValid || mutation.isPending}
          >
            {mutation.isPending ? "Creating..." : "Add Repository"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Edit Repository Modal
// ---------------------------------------------------------------------------

interface RepositoryEditModalProps {
  open: boolean;
  repo: RepositoryRecord;
  onClose: () => void;
}

function RepositoryEditModal({ open, repo, onClose }: RepositoryEditModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [remoteUrl, setRemoteUrl] = useState(repo.repository.remote_url);
  const [defaultBranch, setDefaultBranch] = useState(
    repo.repository.default_branch ?? "",
  );
  const [defaultImage, setDefaultImage] = useState(
    repo.repository.default_image ?? "",
  );

  const mutation = useMutation({
    mutationFn: (params: UpdateRepositoryRequest) =>
      apiClient.updateRepository(repo.name, params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      addToast("Repository updated", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update repository",
        "error",
      );
    },
  });

  const isValid = remoteUrl.trim().length > 0;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      remote_url: remoteUrl.trim(),
      default_branch: defaultBranch.trim() || null,
      default_image: defaultImage.trim() || null,
    });
  }, [remoteUrl, defaultBranch, defaultImage, isValid, mutation]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      onClose();
    }
  }, [mutation.isPending, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title={`Edit ${repo.name}`}>
      <div className={styles.formFields} onKeyDown={handleKeyDown}>
        <Input
          label="Remote URL"
          placeholder="https://github.com/org/repo.git"
          value={remoteUrl}
          onChange={(e) => setRemoteUrl(e.target.value)}
          required
        />
        <Input
          label="Default Branch"
          placeholder="main"
          value={defaultBranch}
          onChange={(e) => setDefaultBranch(e.target.value)}
        />
        <Input
          label="Default Image"
          placeholder="ghcr.io/org/repo:latest"
          value={defaultImage}
          onChange={(e) => setDefaultImage(e.target.value)}
        />
        <div className={styles.formActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={handleClose}
            disabled={mutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSubmit}
            disabled={!isValid || mutation.isPending}
          >
            {mutation.isPending ? "Saving..." : "Save Changes"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Delete Repository Modal
// ---------------------------------------------------------------------------

interface RepositoryDeleteModalProps {
  open: boolean;
  repo: RepositoryRecord;
  onClose: () => void;
}

function RepositoryDeleteModal({ open, repo, onClose }: RepositoryDeleteModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => apiClient.deleteRepository(repo.name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      addToast("Repository deleted", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete repository",
        "error",
      );
    },
  });

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      onClose();
    }
  }, [mutation.isPending, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="Delete Repository">
      <div className={styles.deleteModalContent}>
        <p className={styles.deleteMessage}>
          Are you sure you want to delete this repository?
        </p>
        <p className={styles.deleteRepoName}>{repo.name}</p>
        <div className={styles.deleteActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={handleClose}
            disabled={mutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="danger"
            size="md"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Deleting..." : "Delete"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
