import { useState, useCallback } from "react";
import { Button, Modal, Input } from "@hydra/ui";
import type {
  RepositoryRecord,
  UpdateRepositoryRequest,
  RepoWorkflowConfig,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { PatchWorkflowSection } from "./PatchWorkflowSection";
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
  const [reviewerAssignees, setReviewerAssignees] = useState<string[]>(
    repo.repository.patch_workflow?.review_requests?.map((r) => r.assignee) ??
      [],
  );
  const [mergeAssignee, setMergeAssignee] = useState(
    repo.repository.patch_workflow?.merge_request?.assignee ?? "",
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
      remote_url: remoteUrl.trim(),
      default_branch: defaultBranch.trim() || null,
      default_image: defaultImage.trim() || null,
      patch_workflow,
    });
  }, [
    remoteUrl,
    defaultBranch,
    defaultImage,
    reviewerAssignees,
    mergeAssignee,
    isValid,
    mutation,
  ]);

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
