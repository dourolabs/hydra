import { useState, useCallback } from "react";
import { Button, Modal, Input, Textarea } from "@hydra/ui";
import type { AgentRecord, SessionSettings, UpsertAgentRequest } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { SecretsSelector } from "./SecretsSelector";
import {
  AgentSessionSettingsFields,
  collapseAgentSessionSettings,
} from "./AgentSessionSettingsFields";
import {
  parseSimultaneousCap,
  SIMULTANEOUS_HEADLESS_HELP,
  SIMULTANEOUS_INTERACTIVE_HELP,
  SIMULTANEOUS_PLACEHOLDER,
} from "./simultaneousCaps";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
import styles from "./AgentsSection.module.css";

interface AgentCreateModalProps {
  open: boolean;
  onClose: () => void;
  agents: AgentRecord[];
}

export function AgentCreateModal({ open, onClose, agents }: AgentCreateModalProps) {
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [maxTries, setMaxTries] = useState("3");
  const [maxSimultaneousInteractive, setMaxSimultaneousInteractive] = useState("");
  const [maxSimultaneousHeadless, setMaxSimultaneousHeadless] = useState("");
  const [isDefaultConversationAgent, setIsDefaultConversationAgent] = useState(false);
  const [mcpConfigPath, setMcpConfigPath] = useState("");
  const [selectedSecrets, setSelectedSecrets] = useState<string[]>([]);
  const [sessionSettings, setSessionSettings] = useState<SessionSettings>({});

  const resetForm = useCallback(() => {
    setName("");
    setPrompt("");
    setMaxTries("3");
    setMaxSimultaneousInteractive("");
    setMaxSimultaneousHeadless("");
    setIsDefaultConversationAgent(false);
    setMcpConfigPath("");
    setSelectedSecrets([]);
    setSessionSettings({});
  }, []);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<UpsertAgentRequest, unknown>({
    mutationFn: (params) => apiClient.createAgent(params),
    invalidateKeys: [["agents"]],
    successMessage: "Agent created",
    onSuccess: () => {
      resetForm();
      onClose();
    },
  });

  const existingDefaultConversationAgent = agents.find(
    (a) => a.is_default_conversation_agent,
  );
  const defaultConversationConflict =
    isDefaultConversationAgent && existingDefaultConversationAgent != null;

  const isValid =
    name.trim().length > 0 &&
    prompt.trim().length > 0 &&
    !defaultConversationConflict;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    const trimmedName = name.trim();
    mutation.mutate({
      name: trimmedName,
      prompt: prompt.trim(),
      prompt_path: `/agents/${trimmedName}/prompt.md`,
      mcp_config_path: mcpConfigPath.trim() || null,
      mcp_config: null,
      max_tries: parseInt(maxTries, 10) || 3,
      max_simultaneous_interactive: parseSimultaneousCap(maxSimultaneousInteractive),
      max_simultaneous_headless: parseSimultaneousCap(maxSimultaneousHeadless),
      is_default_conversation_agent: isDefaultConversationAgent,
      secrets: selectedSecrets,
      session_settings: collapseAgentSessionSettings(sessionSettings),
    });
  }, [name, prompt, mcpConfigPath, maxTries, maxSimultaneousInteractive, maxSimultaneousHeadless, isDefaultConversationAgent, selectedSecrets, sessionSettings, isValid, mutation]);

  return (
    <Modal open={open} onClose={() => handleClose(onClose, resetForm)} title="Add Agent">
      <div className={sharedStyles.formFields} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
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
        <div className={styles.fieldGroup}>
          <Input
            label="Max simultaneous interactive sessions"
            placeholder={SIMULTANEOUS_PLACEHOLDER}
            value={maxSimultaneousInteractive}
            onChange={(e) => setMaxSimultaneousInteractive(e.target.value)}
            type="number"
            min={0}
            data-testid="agent-create-max-simultaneous-interactive"
          />
          <span className={styles.fieldHelp}>{SIMULTANEOUS_INTERACTIVE_HELP}</span>
        </div>
        <div className={styles.fieldGroup}>
          <Input
            label="Max simultaneous headless sessions"
            placeholder={SIMULTANEOUS_PLACEHOLDER}
            value={maxSimultaneousHeadless}
            onChange={(e) => setMaxSimultaneousHeadless(e.target.value)}
            type="number"
            min={0}
            data-testid="agent-create-max-simultaneous-headless"
          />
          <span className={styles.fieldHelp}>{SIMULTANEOUS_HEADLESS_HELP}</span>
        </div>
        <label className={styles.checkboxLabel}>
          <input
            type="checkbox"
            checked={isDefaultConversationAgent}
            onChange={(e) => setIsDefaultConversationAgent(e.target.checked)}
          />
          Default Conversation Agent
        </label>
        {defaultConversationConflict && (
          <p className={styles.fieldError}>
            &quot;{existingDefaultConversationAgent.name}&quot; is already the
            default conversation agent. Only one agent can be the default
            conversation agent at a time.
          </p>
        )}
        <SecretsSelector
          selected={selectedSecrets}
          onChange={setSelectedSecrets}
        />
        <AgentSessionSettingsFields
          testIdPrefix="agent-create-form"
          value={sessionSettings}
          onChange={setSessionSettings}
        />
        <div className={sharedStyles.formActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={() => handleClose(onClose, resetForm)}
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
            {isPending ? "Creating..." : "Add Agent"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
