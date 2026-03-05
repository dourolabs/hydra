import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Button, Modal, Input, Textarea } from "@metis/ui";
import type {
  RepositoryRecord,
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
  RepoWorkflowConfig,
  AgentRecord,
  UpsertAgentRequest,
} from "@metis/api";
import { apiClient } from "../api/client";
import { useRepositories } from "../hooks/useRepositories";
import { useAgents } from "../hooks/useAgents";
import { useToast } from "../features/toast/useToast";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const { data: repositories, isLoading, error } = useRepositories();
  const [createOpen, setCreateOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<RepositoryRecord | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RepositoryRecord | null>(null);

  const {
    data: agents,
    isLoading: agentsLoading,
    error: agentsError,
  } = useAgents();
  const [agentCreateOpen, setAgentCreateOpen] = useState(false);
  const [agentEditTarget, setAgentEditTarget] = useState<AgentRecord | null>(
    null,
  );
  const [agentDeleteTarget, setAgentDeleteTarget] =
    useState<AgentRecord | null>(null);

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
          <div className={styles.repoList}>
            {repositories.map((repo) => (
              <RepositoryRow
                key={repo.name}
                repo={repo}
                onEdit={() => setEditTarget(repo)}
                onDelete={() => setDeleteTarget(repo)}
              />
            ))}
          </div>
        </Panel>
      )}

      <div className={styles.agentHeaderRow}>
        <span className={styles.sectionTitle}>Agents</span>
        <Button
          variant="primary"
          size="sm"
          onClick={() => setAgentCreateOpen(true)}
        >
          Add Agent
        </Button>
      </div>

      {agentsLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {agentsError && (
        <p className={styles.error}>
          Failed to load agents: {(agentsError as Error).message}
        </p>
      )}

      {agents && agents.length === 0 && (
        <p className={styles.empty}>No agents configured.</p>
      )}

      {agents && agents.length > 0 && (
        <Panel>
          <div className={styles.repoList}>
            {agents.map((agent) => (
              <AgentRow
                key={agent.name}
                agent={agent}
                onEdit={() => setAgentEditTarget(agent)}
                onDelete={() => setAgentDeleteTarget(agent)}
              />
            ))}
          </div>
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

      <AgentCreateModal
        open={agentCreateOpen}
        onClose={() => setAgentCreateOpen(false)}
        agents={agents ?? []}
      />

      {agentEditTarget && (
        <AgentEditModal
          open={!!agentEditTarget}
          agent={agentEditTarget}
          onClose={() => setAgentEditTarget(null)}
          agents={agents ?? []}
        />
      )}

      {agentDeleteTarget && (
        <AgentDeleteModal
          open={!!agentDeleteTarget}
          agent={agentDeleteTarget}
          onClose={() => setAgentDeleteTarget(null)}
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
  const [expanded, setExpanded] = useState(false);

  const pw = repo.repository.patch_workflow;
  const reviewerCount = pw?.review_requests?.length ?? 0;
  const hasMerge = !!pw?.merge_request?.assignee;
  const parts: string[] = [];
  if (reviewerCount > 0) {
    parts.push(`${reviewerCount} reviewer${reviewerCount === 1 ? "" : "s"}`);
  }
  if (hasMerge) {
    parts.push("merge");
  }
  const workflowSummary = parts.length > 0 ? parts.join(", ") : null;

  return (
    <div className={styles.repoItem}>
      <button
        type="button"
        className={styles.repoHeader}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={styles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={styles.repoName}>{repo.name}</span>
        <div className={styles.rowActions}>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onEdit();
            }}
          >
            Edit
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
          >
            Delete
          </Button>
        </div>
      </button>
      {expanded && (
        <div className={styles.repoDetails}>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Remote URL</span>
            <span className={styles.detailValueMono}>
              {repo.repository.remote_url}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Default Branch</span>
            <span className={styles.detailValue}>
              {repo.repository.default_branch ?? (
                <span className={styles.dimText}>—</span>
              )}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Default Image</span>
            <span className={styles.detailValueMono}>
              {repo.repository.default_image ?? (
                <span className={styles.dimText}>—</span>
              )}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Patch Workflow</span>
            <span className={styles.detailValue}>
              {workflowSummary ?? <span className={styles.dimText}>—</span>}
            </span>
          </div>
        </div>
      )}
    </div>
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
        <PatchWorkflowSection
          reviewerAssignees={reviewerAssignees}
          onReviewerAssigneesChange={setReviewerAssignees}
          mergeAssignee={mergeAssignee}
          onMergeAssigneeChange={setMergeAssignee}
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
  const [reviewerAssignees, setReviewerAssignees] = useState<string[]>(
    repo.repository.patch_workflow?.review_requests?.map((r) => r.assignee) ??
      [],
  );
  const [mergeAssignee, setMergeAssignee] = useState(
    repo.repository.patch_workflow?.merge_request?.assignee ?? "",
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
        <PatchWorkflowSection
          reviewerAssignees={reviewerAssignees}
          onReviewerAssigneesChange={setReviewerAssignees}
          mergeAssignee={mergeAssignee}
          onMergeAssigneeChange={setMergeAssignee}
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
// Patch Workflow Form Section (shared between Create and Edit modals)
// ---------------------------------------------------------------------------

interface PatchWorkflowSectionProps {
  reviewerAssignees: string[];
  onReviewerAssigneesChange: (assignees: string[]) => void;
  mergeAssignee: string;
  onMergeAssigneeChange: (value: string) => void;
}

function PatchWorkflowSection({
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

// ---------------------------------------------------------------------------
// Agent Row
// ---------------------------------------------------------------------------

interface AgentRowProps {
  agent: AgentRecord;
  onEdit: () => void;
  onDelete: () => void;
}

function AgentRow({ agent, onEdit, onDelete }: AgentRowProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={styles.repoItem}>
      <button
        type="button"
        className={styles.repoHeader}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={styles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={styles.repoName}>{agent.name}</span>
        {agent.is_assignment_agent && (
          <span className={styles.assignmentBadge}>assignment</span>
        )}
        <div className={styles.rowActions}>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onEdit();
            }}
          >
            Edit
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
          >
            Delete
          </Button>
        </div>
      </button>
      {expanded && (
        <div className={styles.repoDetails}>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Prompt Path</span>
            <span className={styles.detailValueMono}>
              {agent.prompt_path || <span className={styles.dimText}>—</span>}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Max Tries</span>
            <span className={styles.detailValue}>{agent.max_tries}</span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Max Simultaneous</span>
            <span className={styles.detailValue}>
              {agent.max_simultaneous}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Assignment Agent</span>
            <span className={styles.detailValue}>
              {agent.is_assignment_agent ? "Yes" : "No"}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Create Agent Modal
// ---------------------------------------------------------------------------

interface AgentCreateModalProps {
  open: boolean;
  onClose: () => void;
  agents: AgentRecord[];
}

function AgentCreateModal({ open, onClose, agents }: AgentCreateModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [maxTries, setMaxTries] = useState("3");
  const [maxSimultaneous, setMaxSimultaneous] = useState("1");
  const [isAssignmentAgent, setIsAssignmentAgent] = useState(false);

  const resetForm = useCallback(() => {
    setName("");
    setPrompt("");
    setMaxTries("3");
    setMaxSimultaneous("1");
    setIsAssignmentAgent(false);
  }, []);

  const mutation = useMutation({
    mutationFn: (params: UpsertAgentRequest) => apiClient.createAgent(params),
    onSuccess: () => {
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      addToast("Agent created", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to create agent",
        "error",
      );
    },
  });

  const existingAssignmentAgent = agents.find((a) => a.is_assignment_agent);
  const assignmentConflict =
    isAssignmentAgent && existingAssignmentAgent != null;

  const isValid =
    name.trim().length > 0 &&
    prompt.trim().length > 0 &&
    !assignmentConflict;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    const trimmedName = name.trim();
    mutation.mutate({
      name: trimmedName,
      prompt: prompt.trim(),
      prompt_path: `/agents/${trimmedName}/prompt.md`,
      max_tries: parseInt(maxTries, 10) || 3,
      max_simultaneous: parseInt(maxSimultaneous, 10) || 1,
      is_assignment_agent: isAssignmentAgent,
    });
  }, [name, prompt, maxTries, maxSimultaneous, isAssignmentAgent, isValid, mutation]);

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
    <Modal open={open} onClose={handleClose} title="Add Agent">
      <div className={styles.formFields} onKeyDown={handleKeyDown}>
        <Input
          label="Name"
          placeholder="swe"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
        />
        <Textarea
          label="Prompt"
          placeholder="Enter the agent prompt..."
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          rows={6}
          required
        />
        <Input
          label="Max Tries"
          placeholder="3"
          value={maxTries}
          onChange={(e) => setMaxTries(e.target.value)}
          type="number"
        />
        <Input
          label="Max Simultaneous"
          placeholder="1"
          value={maxSimultaneous}
          onChange={(e) => setMaxSimultaneous(e.target.value)}
          type="number"
        />
        <label className={styles.checkboxLabel}>
          <input
            type="checkbox"
            checked={isAssignmentAgent}
            onChange={(e) => setIsAssignmentAgent(e.target.checked)}
          />
          Assignment Agent
        </label>
        {assignmentConflict && (
          <p className={styles.fieldError}>
            &quot;{existingAssignmentAgent.name}&quot; is already the assignment
            agent. Only one agent can be the assignment agent at a time.
          </p>
        )}
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
            {mutation.isPending ? "Creating..." : "Add Agent"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Edit Agent Modal
// ---------------------------------------------------------------------------

interface AgentEditModalProps {
  open: boolean;
  agent: AgentRecord;
  onClose: () => void;
  agents: AgentRecord[];
}

function AgentEditModal({
  open,
  agent,
  onClose,
  agents,
}: AgentEditModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [prompt, setPrompt] = useState(agent.prompt);
  const [maxTries, setMaxTries] = useState(String(agent.max_tries));
  const [maxSimultaneous, setMaxSimultaneous] = useState(
    String(agent.max_simultaneous),
  );
  const [isAssignmentAgent, setIsAssignmentAgent] = useState(
    agent.is_assignment_agent,
  );

  const mutation = useMutation({
    mutationFn: (params: UpsertAgentRequest) =>
      apiClient.updateAgent(agent.name, params),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      addToast("Agent updated", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update agent",
        "error",
      );
    },
  });

  const existingAssignmentAgent = agents.find(
    (a) => a.is_assignment_agent && a.name !== agent.name,
  );
  const assignmentConflict =
    isAssignmentAgent && existingAssignmentAgent != null;

  const isValid = prompt.trim().length > 0 && !assignmentConflict;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      name: agent.name,
      prompt: prompt.trim(),
      prompt_path: agent.prompt_path,
      max_tries: parseInt(maxTries, 10) || 3,
      max_simultaneous: parseInt(maxSimultaneous, 10) || 1,
      is_assignment_agent: isAssignmentAgent,
    });
  }, [agent.name, agent.prompt_path, prompt, maxTries, maxSimultaneous, isAssignmentAgent, isValid, mutation]);

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
    <Modal open={open} onClose={handleClose} title={`Edit ${agent.name}`}>
      <div className={styles.formFields} onKeyDown={handleKeyDown}>
        <Textarea
          label="Prompt"
          placeholder="Enter the agent prompt..."
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          rows={6}
          required
        />
        <Input
          label="Max Tries"
          placeholder="3"
          value={maxTries}
          onChange={(e) => setMaxTries(e.target.value)}
          type="number"
        />
        <Input
          label="Max Simultaneous"
          placeholder="1"
          value={maxSimultaneous}
          onChange={(e) => setMaxSimultaneous(e.target.value)}
          type="number"
        />
        <label className={styles.checkboxLabel}>
          <input
            type="checkbox"
            checked={isAssignmentAgent}
            onChange={(e) => setIsAssignmentAgent(e.target.checked)}
          />
          Assignment Agent
        </label>
        {assignmentConflict && (
          <p className={styles.fieldError}>
            &quot;{existingAssignmentAgent.name}&quot; is already the assignment
            agent. Only one agent can be the assignment agent at a time.
          </p>
        )}
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
// Delete Agent Modal
// ---------------------------------------------------------------------------

interface AgentDeleteModalProps {
  open: boolean;
  agent: AgentRecord;
  onClose: () => void;
}

function AgentDeleteModal({ open, agent, onClose }: AgentDeleteModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => apiClient.deleteAgent(agent.name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      addToast("Agent deleted", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete agent",
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
    <Modal open={open} onClose={handleClose} title="Delete Agent">
      <div className={styles.deleteModalContent}>
        <p className={styles.deleteMessage}>
          Are you sure you want to delete this agent?
        </p>
        <p className={styles.deleteRepoName}>{agent.name}</p>
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
