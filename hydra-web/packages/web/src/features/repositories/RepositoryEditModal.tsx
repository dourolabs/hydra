import { useState, useMemo, useCallback } from "react";
import { Button, Modal, Input, Textarea } from "@hydra/ui";
import type { MergePolicy, RepositoryRecord, UpdateRepositoryRequest } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
import styles from "./RepositoryEditModal.module.css";

interface RepositoryEditModalProps {
  open: boolean;
  repo: RepositoryRecord;
  onClose: () => void;
}

const MERGE_POLICY_PLACEHOLDER = `{
  "reviewers": [
    { "any_of": ["users/alice"], "count": 1 }
  ],
  "mergers": { "any_of": ["@patch.author"] }
}`;

function initialMergePolicyText(policy: MergePolicy | null | undefined): string {
  if (!policy) return "";
  return JSON.stringify(policy, null, 2);
}

interface ParsedPolicy {
  value: MergePolicy | null;
  error: string | null;
}

function parseMergePolicy(text: string): ParsedPolicy {
  const trimmed = text.trim();
  if (trimmed.length === 0) {
    return { value: null, error: null };
  }
  try {
    const parsed = JSON.parse(trimmed) as MergePolicy;
    return { value: parsed, error: null };
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    return { value: null, error: `Invalid JSON: ${message}` };
  }
}

export function RepositoryEditModal({ open, repo, onClose }: RepositoryEditModalProps) {
  const [remoteUrl, setRemoteUrl] = useState(repo.repository.remote_url);
  const [defaultBranch, setDefaultBranch] = useState(repo.repository.default_branch ?? "");
  const [defaultImage, setDefaultImage] = useState(repo.repository.default_image ?? "");
  const [mergePolicyText, setMergePolicyText] = useState(() =>
    initialMergePolicyText(repo.repository.merge_policy),
  );

  const parsedPolicy = useMemo(() => parseMergePolicy(mergePolicyText), [mergePolicyText]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    UpdateRepositoryRequest,
    unknown
  >({
    mutationFn: (params) => apiClient.updateRepository(repo.name, params),
    invalidateKeys: [["repositories"]],
    successMessage: "Repository updated",
    onSuccess: () => {
      onClose();
    },
  });

  const isValid = remoteUrl.trim().length > 0 && parsedPolicy.error === null;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      remote_url: remoteUrl.trim(),
      default_branch: defaultBranch.trim() || null,
      default_image: defaultImage.trim() || null,
      merge_policy: parsedPolicy.value,
    });
  }, [remoteUrl, defaultBranch, defaultImage, parsedPolicy.value, isValid, mutation]);

  const handleClearPolicy = useCallback(() => {
    setMergePolicyText("");
  }, []);

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
        <div className={styles.policyField}>
          <div className={styles.policyHeader}>
            <span className={styles.policyLabel}>Merge Policy (JSON)</span>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleClearPolicy}
              disabled={isPending || mergePolicyText.length === 0}
              data-testid="merge-policy-clear"
            >
              Clear policy
            </Button>
          </div>
          <Textarea
            id="merge-policy-editor"
            placeholder={MERGE_POLICY_PLACEHOLDER}
            value={mergePolicyText}
            onChange={(e) => setMergePolicyText(e.target.value)}
            error={parsedPolicy.error ?? undefined}
            rows={10}
            spellCheck={false}
            className={styles.policyTextarea}
            data-testid="merge-policy-editor"
          />
        </div>
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
