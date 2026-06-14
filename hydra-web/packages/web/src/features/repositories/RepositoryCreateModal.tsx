import { useState, useCallback } from "react";
import { Button, Modal, Input } from "@hydra/ui";
import type { CreateRepositoryRequest } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";

interface RepositoryCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function RepositoryCreateModal({ open, onClose }: RepositoryCreateModalProps) {
  const [name, setName] = useState("");
  const [remoteUrl, setRemoteUrl] = useState("");
  const [defaultBranch, setDefaultBranch] = useState("");

  const resetForm = useCallback(() => {
    setName("");
    setRemoteUrl("");
    setDefaultBranch("");
  }, []);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<CreateRepositoryRequest, unknown>({
    mutationFn: (params) => apiClient.createRepository(params),
    invalidateKeys: [["repositories"]],
    successMessage: "Repository created",
    onSuccess: () => {
      resetForm();
      onClose();
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
    });
  }, [name, remoteUrl, defaultBranch, isValid, mutation]);

  return (
    <Modal open={open} onClose={() => handleClose(onClose, resetForm)} title="Add Repository">
      <div className={sharedStyles.formFields} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
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
        <div className={sharedStyles.formActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={() => handleClose(onClose, resetForm)}
            disabled={isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSubmit}
            disabled={!isValid || isPending}
          >
            {isPending ? "Creating..." : "Add Repository"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
