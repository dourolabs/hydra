import { useCallback } from "react";
import { Button, Input } from "@hydra/ui";
import styles from "./RepositoriesSection.module.css";

interface PatchWorkflowSectionProps {
  reviewerAssignees: string[];
  onReviewerAssigneesChange: (assignees: string[]) => void;
  mergeAssignee: string;
  onMergeAssigneeChange: (value: string) => void;
}

export function PatchWorkflowSection({
  reviewerAssignees,
  onReviewerAssigneesChange,
  mergeAssignee,
  onMergeAssigneeChange,
}: PatchWorkflowSectionProps) {
  const addReviewer = useCallback(() => {
    onReviewerAssigneesChange([...reviewerAssignees, ""]);
  }, [reviewerAssignees, onReviewerAssigneesChange]);

  const removeReviewer = useCallback(
    (index: number) => {
      onReviewerAssigneesChange(reviewerAssignees.filter((_, i) => i !== index));
    },
    [reviewerAssignees, onReviewerAssigneesChange],
  );

  const updateReviewer = useCallback(
    (index: number, value: string) => {
      const updated = [...reviewerAssignees];
      updated[index] = value;
      onReviewerAssigneesChange(updated);
    },
    [reviewerAssignees, onReviewerAssigneesChange],
  );

  return (
    <div className={styles.workflowSection}>
      <div className={styles.workflowHeader}>Patch Workflow</div>
      <p className={styles.formHint}>
        Use $patch_creator to auto-assign to the patch author
      </p>
      <div className={styles.reviewerList}>
        {reviewerAssignees.map((assignee, index) => (
          <div key={index} className={styles.reviewerRow}>
            <Input
              label={index === 0 ? "Review Assignees" : undefined}
              placeholder="reviewer username or $patch_creator"
              value={assignee}
              onChange={(e) => updateReviewer(index, e.target.value)}
            />
            <Button
              variant="ghost"
              size="sm"
              onClick={() => removeReviewer(index)}
            >
              Remove
            </Button>
          </div>
        ))}
        <Button variant="secondary" size="sm" onClick={addReviewer}>
          Add Reviewer
        </Button>
      </div>
      <Input
        label="Merge Request Assignee"
        placeholder="username or $patch_creator"
        value={mergeAssignee}
        onChange={(e) => onMergeAssigneeChange(e.target.value)}
      />
    </div>
  );
}
