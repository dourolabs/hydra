import { useState, useCallback } from "react";
import { Button, Modal, Input, Textarea } from "@hydra/ui";
import type { AgentRecord, UpsertAgentRequest } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { SecretsSelector } from "./SecretsSelector";
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
  const [prompt, setPrompt] = useState(agent.prompt);
  const [mcpConfigPath, setMcpConfigPath] = useState(agent.mcp_config_path ?? "");
  const [maxTries, setMaxTries] = useState(String(agent.max_tries));
  const [maxSimultaneous, setMaxSimultaneous] = useState(
    String(agent.max_simultaneous),
  );
  const [isAssignmentAgent, setIsAssignmentAgent] = useState(
    agent.is_assignment_agent,
  );
  const [selectedSecrets, setSelectedSecrets] = useState<string[]>(
    agent.secrets ?? [],
  );

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<UpsertAgentRequest, unknown>({
    mutationFn: (params) => apiClient.updateAgent(agent.name, params),
    invalidateKeys: [["agents"]],
    successMessage: "Agent updated",
    onSuccess: () => {
      onClose();
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
      mcp_config_path: mcpConfigPath.trim() || null,
      mcp_config: null,
      max_tries: parseInt(maxTries, 10) || 3,
      max_simultaneous: parseInt(maxSimultaneous, 10) || 1,
      is_assignment_agent: isAssignmentAgent,
      secrets: selectedSecrets,
    });
  }, [agent.name, agent.prompt_path, mcpConfigPath, prompt, maxTries, maxSimultaneous, isAssignmentAgent, selectedSecrets, isValid, mutation]);

  return (
    <Modal open={open} onClose={() => handleClose(onClose)} title={`Edit ${agent.name}`}>
      <div className={styles.formFields} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
        <Textarea
          label="Prompt"
          placeholder="Enter the agent prompt..."
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          rows={6}
          required
        />
        <Input
          label="MCP Config Path"
          placeholder="/agents/my-agent/mcp-config.json"
          value={mcpConfigPath}
          onChange={(e) => setMcpConfigPath(e.target.value)}
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
        <SecretsSelector
          selected={selectedSecrets}
          onChange={setSelectedSecrets}
        />
        <div className={styles.formActions}>
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
