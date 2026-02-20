import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Textarea, Select } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { RepositoryRecord } from "@metis/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useFormDraft } from "../../hooks/useFormDraft";
import { useAuth } from "../auth/useAuth";
import { useToast } from "../toast/useToast";
import { actorDisplayName } from "../../api/auth";
import styles from "./IssueCreator.module.css";

function buildRepoOptions(repos: RepositoryRecord[] | undefined): SelectOption[] {
  const options: SelectOption[] = [{ value: "", label: "None" }];
  if (repos) {
    for (const r of repos) {
      options.push({ value: r.name, label: r.name });
    }
  }
  return options;
}

interface IssueCreatorProps {
  assignees: string[];
}

export function IssueCreator({ assignees }: IssueCreatorProps) {
  const { user } = useAuth();
  const { addToast } = useToast();
  const currentUsername = user ? actorDisplayName(user.actor) : "";

  const [description, setDescription, clearDescriptionDraft] = useFormDraft("metis:draft:issue-creator:description", "");
  const [assignee, setAssignee, clearAssigneeDraft] = useFormDraft("metis:draft:issue-creator:assignee", "");
  const [repoName, setRepoName, clearRepoNameDraft] = useFormDraft("metis:draft:issue-creator:repoName", "");
  const [showOptions, setShowOptions] = useState(false);

  const queryClient = useQueryClient();
  const { data: repos } = useRepositories();

  const mutation = useMutation({
    mutationFn: (params: { description: string; creator: string; assignee?: string; repoName?: string }) =>
      apiClient.createIssue({
        issue: {
          type: "task",
          description: params.description,
          creator: params.creator,
          progress: "",
          status: "open",
          dependencies: [],
          patches: [],
          ...(params.assignee && { assignee: params.assignee }),
          ...(params.repoName && { job_settings: { repo_name: params.repoName } }),
        },
        job_id: null,
      }),
    onSuccess: (data) => {
      setDescription("");
      setAssignee("");
      setRepoName("");
      clearDescriptionDraft();
      clearAssigneeDraft();
      clearRepoNameDraft();
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      addToast(`Issue ${data.issue_id} created`, "success");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to create issue",
        "error",
      );
    },
  });

  const handleSubmit = () => {
    const desc = description.trim();
    if (!desc) return;

    mutation.mutate({
      description: desc,
      creator: currentUsername,
      ...(assignee && { assignee }),
      ...(repoName && { repoName }),
    });
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      handleSubmit();
    }
  };

  const assigneeOptions: SelectOption[] = [
    { value: "", label: "Unassigned" },
    ...assignees.map((a) => ({ value: a, label: a })),
  ];

  return (
    <div className={styles.creator}>
      <Textarea
        placeholder="Create a new issue..."
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        onKeyDown={handleKeyDown}
        rows={2}
        className={styles.textarea}
      />
      {showOptions && (
        <div className={styles.options}>
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
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.toggleOptions}
          onClick={() => setShowOptions(!showOptions)}
        >
          {showOptions ? "Hide options" : "Options"}
        </button>
        <Button
          variant="primary"
          size="sm"
          onClick={handleSubmit}
          disabled={!description.trim() || mutation.isPending}
        >
          {mutation.isPending ? "Creating..." : "Create issue"}
        </Button>
      </div>
    </div>
  );
}
