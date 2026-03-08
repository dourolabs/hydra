import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Button, Modal, Input, Textarea } from "@metis/ui";
import type { AgentRecord, UpsertAgentRequest } from "@metis/api";
import { apiClient } from "../../api/client";
import { useAgents } from "../../hooks/useAgents";
import { useToast } from "../toast/useToast";
import styles from "./AgentsSection.module.css";

export function AgentsSection() {
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
    <>
      <div className={styles.headerRow}>
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
          <div className={styles.agentList}>
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
    </>
  );
}

interface AgentRowProps {
  agent: AgentRecord;
  onEdit: () => void;
  onDelete: () => void;
}

function AgentRow({ agent, onEdit, onDelete }: AgentRowProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={styles.agentItem}>
      <button
        type="button"
        className={styles.agentHeader}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={styles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={styles.agentName}>{agent.name}</span>
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
        <div className={styles.agentDetails}>
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
        <p className={styles.deleteAgentName}>{agent.name}</p>
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
