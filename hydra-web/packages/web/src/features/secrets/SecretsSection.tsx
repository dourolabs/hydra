import { useState } from "react";
import { Button, Panel, Spinner } from "@hydra/ui";
import { useUsername } from "../auth/useUsername";
import { useSecrets } from "./useSecrets";
import { SecretRow } from "./SecretRow";
import { AddSecretForm } from "./AddSecretForm";
import styles from "./SecretsSection.module.css";

const KNOWN_SECRETS = [
  {
    name: "GH_TOKEN",
    label: "GitHub Token",
    description:
      "Automatically provided from your GitHub login. You can override it below if needed.",
  },
  { name: "OPENAI_API_KEY", label: "OpenAI API Key" },
  { name: "ANTHROPIC_API_KEY", label: "Anthropic API Key" },
  { name: "CLAUDE_CODE_OAUTH_TOKEN", label: "Claude Code OAuth Token" },
];

export function SecretsSection() {
  const username = useUsername();
  const { data, isLoading, error } = useSecrets(username);
  const configuredSecrets = data?.secrets ?? [];
  const knownSecretNames = new Set(KNOWN_SECRETS.map((s) => s.name));
  const customSecrets = configuredSecrets.filter((n) => !knownSecretNames.has(n));
  const [adding, setAdding] = useState(false);

  return (
    <div>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && <p className={styles.error}>Failed to load secrets: {(error as Error).message}</p>}

      {data && (
        <Panel
          header={
            <div className={styles.panelHeaderRow}>
              <span className={styles.sectionTitle}>Secrets</span>
              <Button variant="primary" size="sm" onClick={() => setAdding(true)}>
                Add Secret
              </Button>
            </div>
          }
        >
          <div className={styles.secretList}>
            {KNOWN_SECRETS.map((secret) => (
              <SecretRow
                key={secret.name}
                username={username!}
                name={secret.name}
                label={secret.label}
                description={secret.description}
                configured={configuredSecrets.includes(secret.name)}
              />
            ))}
            {customSecrets.map((name) => (
              <SecretRow
                key={name}
                username={username!}
                name={name}
                label="Custom secret"
                configured={true}
              />
            ))}
          </div>
          <AddSecretForm
            username={username!}
            existingNames={configuredSecrets}
            adding={adding}
            onClose={() => setAdding(false)}
          />
        </Panel>
      )}
    </div>
  );
}
