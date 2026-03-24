import { useState, useCallback } from "react";
import { Button, Modal, Input } from "@hydra/ui";
import type {
  CreateRepositoryRequest,
  RepoWorkflowConfig,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { PatchWorkflowSection } from "./PatchWorkflowSection";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";

interface RepositoryCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function RepositoryCreateModal({ open, onClose }: RepositoryCreateModalProps) {
  const [name, setName] = useState("");
  const [remoteUrl, setRemoteUrl] = useState("");
  const [defaultBranch, setDefaultBranch] = useState("");
  const [defaultImage, setDefaultImage] = useState("");
  const [reviewerAssignees, setReviewerAssignees] = useState<string[]>([]);
  const [mergeAssignee, setMergeAssignee] = useState("");

  const resetForm = useCallback(() => {
    setName("");
    setRemoteUrl("");
    setDefaultBranch("");
    setDefaultImage("");
    setReviewerAssignees([]);
    setMergeAssignee("");
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
    const filteredReviewers = reviewerAssignees
      .map((r) => r.trim())
      .filter((r) => r.length > 0);
    const trimmedMergeAssignee = mergeAssignee.trim();
    const hasPatchWorkflow =
      filteredReviewers.length > 0 || trimmedMergeAssignee.length > 0;
    const patch_workflow: RepoWorkflowConfig | undefined = hasPatchWorkflow
      ? {
          review_requests: filteredReviewers.map((assignee) => ({ assignee })),
          merge_request: trimmedMergeAssignee
            ? { assignee: trimmedMergeAssignee }
            : null,
        }
      : undefined;
    mutation.mutate({
      name: name.trim(),
      remote_url: remoteUrl.trim(),
      default_branch: defaultBranch.trim() || null,
      default_image: defaultImage.trim() || null,
      patch_workflow,
    });
  }, [
    name,
    remoteUrl,
    defaultBranch,
    defaultImage,
    reviewerAssignees,
    mergeAssignee,
    isValid,
    mutation,
  ]);

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
        <Input
          label="Default Image"
          placeholder="ghcr.io/org/repo:latest"
          value={defaultImage}
          onChange={(e) => setDefaultImage(e.target.value)}
        />
        <PatchWorkflowSection
          reviewerAssignees={reviewerAssignees}
          onReviewerAssigneesChange={setReviewerAssignees}
          mergeAssignee={mergeAssignee}
          onMergeAssigneeChange={setMergeAssignee}
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
