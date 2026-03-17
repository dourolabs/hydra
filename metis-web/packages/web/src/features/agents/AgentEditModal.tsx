import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Modal, Input, Textarea } from "@metis/ui";
import type { AgentRecord, UpsertAgentRequest } from "@metis/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./AgentsSection.module.css";

interface AgentEditModalProps {
  open: boolean;
  agent: AgentRecord;
  onClose: () => void;
  agents: AgentRecord[];
}

export function AgentEditModal({
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
      secrets: agent.secrets,
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
