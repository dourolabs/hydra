import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Modal, Input, Textarea } from "@metis/ui";
import type { AgentRecord, UpsertAgentRequest } from "@metis/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./AgentsSection.module.css";

interface AgentCreateModalProps {
  open: boolean;
  onClose: () => void;
  agents: AgentRecord[];
}

export function AgentCreateModal({ open, onClose, agents }: AgentCreateModalProps) {
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
      secrets: [],
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
