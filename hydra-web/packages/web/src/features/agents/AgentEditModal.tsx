import { useState, useCallback } from "react";
import { Button, Modal, Input, Textarea } from "@hydra/ui";
import type { AgentRecord, SessionSettings, UpsertAgentRequest } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { SecretsSelector } from "./SecretsSelector";
import {
  SessionSettingsFields,
  collapseSessionSettings,
} from "../sessions/SessionSettingsFields";
import {
  formatSimultaneousCap,
  parseSimultaneousCap,
  SIMULTANEOUS_HEADLESS_HELP,
  SIMULTANEOUS_INTERACTIVE_HELP,
  SIMULTANEOUS_PLACEHOLDER,
} from "./simultaneousCaps";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
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
  const [maxSimultaneousInteractive, setMaxSimultaneousInteractive] = useState(
    formatSimultaneousCap(agent.max_simultaneous_interactive),
  );
  const [maxSimultaneousHeadless, setMaxSimultaneousHeadless] = useState(
    formatSimultaneousCap(agent.max_simultaneous_headless),
  );
  const [isDefaultConversationAgent, setIsDefaultConversationAgent] = useState(
    agent.is_default_conversation_agent,
  );
  const [selectedSecrets, setSelectedSecrets] = useState<string[]>(
    agent.secrets ?? [],
  );
  const [sessionSettings, setSessionSettings] = useState<SessionSettings>(
    () => agent.session_settings ?? {},
  );

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<UpsertAgentRequest, unknown>({
    mutationFn: (params) => apiClient.updateAgent(agent.name, params),
    invalidateKeys: [["agents"]],
    successMessage: "Agent updated",
    onSuccess: () => {
      onClose();
    },
  });

  const existingDefaultConversationAgent = agents.find(
    (a) => a.is_default_conversation_agent && a.name !== agent.name,
  );
  const defaultConversationConflict =
    isDefaultConversationAgent && existingDefaultConversationAgent != null;

  const isValid =
    prompt.trim().length > 0 && !defaultConversationConflict;

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      name: agent.name,
      prompt: prompt.trim(),
      prompt_path: agent.prompt_path,
      mcp_config_path: mcpConfigPath.trim() || null,
      mcp_config: null,
      max_tries: parseInt(maxTries, 10) || 3,
      max_simultaneous_interactive: parseSimultaneousCap(maxSimultaneousInteractive),
      max_simultaneous_headless: parseSimultaneousCap(maxSimultaneousHeadless),
      is_default_conversation_agent: isDefaultConversationAgent,
      secrets: selectedSecrets,
      session_settings: collapseSessionSettings(sessionSettings),
    });
  }, [agent.name, agent.prompt_path, mcpConfigPath, prompt, maxTries, maxSimultaneousInteractive, maxSimultaneousHeadless, isDefaultConversationAgent, selectedSecrets, sessionSettings, isValid, mutation]);

  return (
    <Modal open={open} onClose={() => handleClose(onClose)} title={`Edit ${agent.name}`}>
      <div className={sharedStyles.formFields} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
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
            data-testid="agent-edit-max-simultaneous-interactive"
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
            data-testid="agent-edit-max-simultaneous-headless"
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
        <SessionSettingsFields
          testIdPrefix="agent-edit-form"
          value={sessionSettings}
          onChange={setSessionSettings}
          helpText="Per-agent defaults applied when spawning sessions for this agent. Issue-, status-, and project-level settings still win over these. Leave blank to inherit the global defaults."
        />
        <div className={sharedStyles.formActions}>
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
