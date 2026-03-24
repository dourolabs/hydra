import { useCallback, useState } from "react";
import { Modal, Button, Input, Textarea, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type { IssueType, RepositoryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useFormDraft } from "../../hooks/useFormDraft";
import { useFormModal } from "../../hooks/useFormModal";
import { useAuth } from "../auth/useAuth";
import { actorDisplayName } from "../../api/auth";
import { LabelPicker } from "../labels/LabelPicker";
import largeModalStyles from "../../components/LargeModal.module.css";
import styles from "./IssueCreateModal.module.css";

const issueTypeOptions: SelectOption[] = [
  { value: "task", label: "Task" },
  { value: "bug", label: "Bug" },
  { value: "feature", label: "Feature" },
  { value: "chore", label: "Chore" },
];

function buildRepoOptions(repos: RepositoryRecord[] | undefined): SelectOption[] {
  const options: SelectOption[] = [{ value: "", label: "None" }];
  if (repos) {
    for (const r of repos) {
      options.push({ value: r.name, label: r.name });
    }
  }
  return options;
}

interface IssueCreateModalProps {
  open: boolean;
  onClose: () => void;
  assignees: string[];
}

export function IssueCreateModal({ open, onClose, assignees }: IssueCreateModalProps) {
  const { user } = useAuth();
  const { data: repos } = useRepositories();
  const currentUsername = user ? actorDisplayName(user.actor) : "";

  const [title, setTitle, clearTitleDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:title",
    "",
  );
  const [description, setDescription, clearDescriptionDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:description",
    "",
  );
  const [issueType, setIssueType, clearIssueTypeDraft] = useFormDraft<IssueType>(
    "hydra:draft:issue-create-modal:issueType",
    "task",
  );
  const [assignee, setAssignee, clearAssigneeDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:assignee",
    "",
  );
  const [repoName, setRepoName, clearRepoNameDraft] = useFormDraft(
    "hydra:draft:issue-create-modal:repoName",
    "",
  );
  const [labelNames, setLabelNames, clearLabelNamesDraft] = useFormDraft<string[]>(
    "hydra:draft:issue-create-modal:labelNames",
    [],
  );
  const [showMoreOptions, setShowMoreOptions] = useState(false);

  const resetForm = useCallback(() => {
    setTitle("");
    setDescription("");
    setIssueType("task");
    setAssignee("");
    setRepoName("");
    setLabelNames([]);
    clearTitleDraft();
    clearDescriptionDraft();
    clearIssueTypeDraft();
    clearAssigneeDraft();
    clearRepoNameDraft();
    clearLabelNamesDraft();
  }, [
    setTitle,
    setDescription,
    setIssueType,
    setAssignee,
    setRepoName,
    setLabelNames,
    clearTitleDraft,
    clearDescriptionDraft,
    clearIssueTypeDraft,
    clearAssigneeDraft,
    clearRepoNameDraft,
    clearLabelNamesDraft,
  ]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    {
      title: string;
      description: string;
      creator: string;
      type: IssueType;
      assignee?: string;
      repoName?: string;
      labelNames?: string[];
    },
    { issue_id: string }
  >({
    mutationFn: (params) =>
      apiClient.createIssue({
        issue: {
          type: params.type,
          title: params.title,
          description: params.description,
          creator: params.creator,
          progress: "",
          status: "open",
          dependencies: [],
          patches: [],
          ...(params.assignee && { assignee: params.assignee }),
          ...(params.repoName && {
            session_settings: { repo_name: params.repoName },
          }),
        },
        session_id: null,
        ...(params.labelNames &&
          params.labelNames.length > 0 && {
            label_names: params.labelNames,
          }),
      }),
    invalidateKeys: [["issues"]],
    successMessage: (data) => `Issue ${data.issue_id} created`,
    onSuccess: () => {
      resetForm();
      onClose();
    },
  });

  const handleSubmit = useCallback(() => {
    const desc = description.trim();
    if (!desc) return;
    mutation.mutate({
      title: title.trim(),
      description: desc,
      creator: currentUsername,
      type: issueType,
      ...(assignee && { assignee }),
      ...(repoName && { repoName }),
      ...(labelNames.length > 0 && { labelNames }),
    });
  }, [title, description, currentUsername, issueType, assignee, repoName, labelNames, mutation]);

  const resetAll = useCallback(() => {
    resetForm();
    setShowMoreOptions(false);
  }, [resetForm]);

  const assigneeOptions: SelectOption[] = [
    { value: "", label: "Unassigned" },
    ...assignees.map((a) => ({ value: a, label: a })),
  ];

  return (
    <Modal
      open={open}
      onClose={() => handleClose(onClose, resetAll)}
      title="Create Issue"
      className={largeModalStyles.largeModal}
    >
      <div className={styles.form} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
        <Input
          label="Title"
          placeholder="Short summary (optional)"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
        />
        <div className={styles.descriptionWrapper}>
          <Textarea
            label="Description"
            placeholder="Describe the issue..."
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className={styles.descriptionTextarea}
          />
        </div>
        {showMoreOptions && (
          <div className={styles.fields}>
            <Select
              label="Type"
              options={issueTypeOptions}
              value={issueType}
              onChange={(e) => setIssueType(e.target.value as IssueType)}
            />
            <Select
              label="Assignee"
              options={assigneeOptions}
              value={assignee}
              onChange={(e) => setAssignee(e.target.value)}
            />
            <Select
              label="Repository"
              options={buildRepoOptions(repos)}
              value={repoName}
              onChange={(e) => setRepoName(e.target.value)}
            />
          </div>
        )}
        <LabelPicker selectedNames={labelNames} onChange={setLabelNames} />
        <div className={styles.footer}>
          <div className={styles.footerLeft}>
            <button
              type="button"
              className={styles.toggleOptions}
              onClick={() => setShowMoreOptions(!showMoreOptions)}
            >
              {showMoreOptions ? "Hide options" : "More options"}
            </button>
            <span className={styles.hint}>
              {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter to submit
            </span>
          </div>
          <div className={styles.footerActions}>
            <Button variant="secondary" size="md" onClick={() => handleClose(onClose, resetAll)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              onClick={handleSubmit}
              disabled={!description.trim() || isPending}
            >
              {isPending ? "Creating..." : "Create Issue"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
