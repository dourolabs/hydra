import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Textarea, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useFormDraft } from "../../hooks/useFormDraft";
import { useIsMobile } from "../../hooks/useIsMobile";
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

export interface IssueCreatorAssignees {
  agents: string[];
  users: string[];
}

interface IssueCreatorProps {
  assignees: IssueCreatorAssignees;
}

function parseAssigneePath(
  path: string,
): { kind: "agent" | "user"; name: string } | null {
  if (!path) return null;
  if (path.startsWith("agents/")) {
    return { kind: "agent", name: path.slice("agents/".length) };
  }
  if (path.startsWith("users/")) {
    return { kind: "user", name: path.slice("users/".length) };
  }
  return null;
}

export function IssueCreator({ assignees }: IssueCreatorProps) {
  const { user } = useAuth();
  const { addToast } = useToast();
  const isMobile = useIsMobile();
  const currentUsername = user ? actorDisplayName(user.actor) : "";

  const [title, setTitle, clearTitleDraft] = useFormDraft("hydra:draft:issue-creator:title", "");
  const [description, setDescription, clearDescriptionDraft] = useFormDraft("hydra:draft:issue-creator:description", "");
  const [assignee, setAssignee, clearAssigneeDraft] = useFormDraft("hydra:draft:issue-creator:assignee", "");
  const [repoName, setRepoName, clearRepoNameDraft] = useFormDraft("hydra:draft:issue-creator:repoName", "");
  const [showOptions, setShowOptions] = useState(false);

  const queryClient = useQueryClient();
  const { data: repos } = useRepositories();

  const mutation = useMutation({
    mutationFn: (params: {
      title: string;
      description: string;
      creator: string;
      assignee?: { kind: "agent" | "user"; name: string };
      repoName?: string;
    }) =>
      apiClient.createIssue({
        issue: {
          type: "task",
          title: params.title,
          description: params.description,
          creator: params.creator,
          progress: "",
          status: "open",
          dependencies: [],
          patches: [],
          ...(params.assignee && { assignee: params.assignee }),
          ...(params.repoName && { session_settings: { repo_name: params.repoName } }),
        },
        session_id: null,
      }),
    onSuccess: (data) => {
      setTitle("");
      setDescription("");
      setAssignee("");
      setRepoName("");
      clearTitleDraft();
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

    const assigneePrincipal = parseAssigneePath(assignee);
    mutation.mutate({
      title: title.trim(),
      description: desc,
      creator: currentUsername,
      ...(assigneePrincipal && { assignee: assigneePrincipal }),
      ...(repoName && { repoName }),
    });
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (isMobile) return;
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      handleSubmit();
    }
  };

  // Section-prefixed labels so the user can tell agents apart from users in
  // the flat `<select>`. The wire value is the Principal path so the kind is
  // recoverable at submit time.
  const assigneeOptions: SelectOption[] = [
    { value: "", label: "Unassigned" },
    ...assignees.agents.map((a) => ({ value: `agents/${a}`, label: `Agent · ${a}` })),
    ...assignees.users.map((u) => ({ value: `users/${u}`, label: `User · ${u}` })),
  ];

  return (
    <div className={styles.creator}>
      <Input
        placeholder="Title (optional)"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={handleKeyDown}
      />
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
