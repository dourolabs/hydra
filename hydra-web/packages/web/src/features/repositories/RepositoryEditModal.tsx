import { useState, useCallback } from "react";
import { Button, Modal, Input } from "@hydra/ui";
import type { RepositoryRecord, UpdateRepositoryRequest } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";

interface RepositoryEditModalProps {
  open: boolean;
  repo: RepositoryRecord;
  onClose: () => void;
}

export function RepositoryEditModal({ open, repo, onClose }: RepositoryEditModalProps) {
  const [remoteUrl, setRemoteUrl] = useState(repo.repository.remote_url);
  const [defaultBranch, setDefaultBranch] = useState(
    repo.repository.default_branch ?? "",
  );
  const [defaultImage, setDefaultImage] = useState(
    repo.repository.default_image ?? "",
  );

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<UpdateRepositoryRequest, unknown>({
    mutationFn: (params) => apiClient.updateRepository(repo.name, params),
    invalidateKeys: [["repositories"]],
    successMessage: "Repository updated",
    onSuccess: () => {
      onClose();
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

  return (
    <Modal open={open} onClose={() => handleClose(onClose)} title={`Edit ${repo.name}`}>
      <div className={sharedStyles.formFields} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
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
        <div className={sharedStyles.formActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={() => handleClose(onClose)}
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
            {isPending ? "Saving..." : "Save Changes"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
