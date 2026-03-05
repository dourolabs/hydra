import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Button, Input } from "@metis/ui";
import { apiClient } from "../../api/client";
import { useSecrets } from "./useSecrets";
import { useToast } from "../toast/useToast";
import styles from "./SecretsSection.module.css";

const KNOWN_SECRETS = [
  { name: "OPENAI_API_KEY", label: "OpenAI API Key" },
  { name: "ANTHROPIC_API_KEY", label: "Anthropic API Key" },
  { name: "CLAUDE_CODE_OAUTH_TOKEN", label: "Claude Code OAuth Token" },
];

export function SecretsSection() {
  const { data, isLoading, error } = useSecrets();
  const configuredSecrets = data?.secrets ?? [];

  return (
    <div>
      <div className={styles.headerRow}>
        <span className={styles.sectionTitle}>Secrets</span>
      </div>

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>
          Failed to load secrets: {(error as Error).message}
        </p>
      )}

      {data && (
        <Panel>
          <div className={styles.secretList}>
            {KNOWN_SECRETS.map((secret) => (
              <SecretRow
                key={secret.name}
                name={secret.name}
                label={secret.label}
                configured={configuredSecrets.includes(secret.name)}
              />
            ))}
          </div>
        </Panel>
      )}
    </div>
  );
}

interface SecretRowProps {
  name: string;
  label: string;
  configured: boolean;
}

function SecretRow({ name, label, configured }: SecretRowProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");

  const setMutation = useMutation({
    mutationFn: (secretValue: string) => apiClient.setSecret(name, secretValue),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["secrets"] });
      addToast(`${name} saved`, "success");
      setEditing(false);
      setValue("");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : `Failed to save ${name}`,
        "error",
      );
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.deleteSecret(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["secrets"] });
      addToast(`${name} deleted`, "success");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : `Failed to delete ${name}`,
        "error",
      );
    },
  });

  const handleSave = useCallback(() => {
    if (value.trim().length === 0) return;
    setMutation.mutate(value.trim());
  }, [value, setMutation]);

  const handleCancel = useCallback(() => {
    setEditing(false);
    setValue("");
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSave();
      } else if (e.key === "Escape") {
        handleCancel();
      }
    },
    [handleSave, handleCancel],
  );

  const isPending = setMutation.isPending || deleteMutation.isPending;

  return (
    <div className={styles.secretItem}>
      <div className={styles.secretHeader}>
        <div className={styles.secretInfo}>
          <span className={styles.secretName}>{name}</span>
          <span className={styles.secretLabel}>{label}</span>
        </div>
        <div className={styles.secretStatus}>
          <span className={configured ? styles.statusConfigured : styles.statusNotSet}>
            {configured ? "Configured" : "Not set"}
          </span>
          <div className={styles.secretActions}>
            {!editing && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setEditing(true)}
                disabled={isPending}
              >
                {configured ? "Update" : "Set"}
              </Button>
            )}
            {configured && !editing && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => deleteMutation.mutate()}
                disabled={isPending}
              >
                Delete
              </Button>
            )}
          </div>
        </div>
      </div>
      {editing && (
        <div className={styles.secretForm} onKeyDown={handleKeyDown}>
          <Input
            type="password"
            placeholder={`Enter ${name}`}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            autoFocus
          />
          <div className={styles.secretFormActions}>
            <Button
              variant="secondary"
              size="sm"
              onClick={handleCancel}
              disabled={setMutation.isPending}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={handleSave}
              disabled={value.trim().length === 0 || setMutation.isPending}
            >
              {setMutation.isPending ? "Saving..." : "Save"}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
