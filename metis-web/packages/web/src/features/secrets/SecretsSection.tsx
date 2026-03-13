import { Panel, Spinner } from "@metis/ui";
import { useSecrets } from "./useSecrets";
import { SecretRow } from "./SecretRow";
import { AddSecretForm } from "./AddSecretForm";
import styles from "./SecretsSection.module.css";

const KNOWN_SECRETS = [
  { name: "GH_TOKEN", label: "GitHub Token", description: "Automatically provided from your GitHub login. You can override it below if needed." },
  { name: "OPENAI_API_KEY", label: "OpenAI API Key" },
  { name: "ANTHROPIC_API_KEY", label: "Anthropic API Key" },
  { name: "CLAUDE_CODE_OAUTH_TOKEN", label: "Claude Code OAuth Token" },
];

export function SecretsSection() {
  const { data, isLoading, error } = useSecrets();
  const configuredSecrets = data?.secrets ?? [];
  const knownSecretNames = new Set(KNOWN_SECRETS.map((s) => s.name));
  const customSecrets = configuredSecrets.filter((n) => !knownSecretNames.has(n));

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
                description={secret.description}
                configured={configuredSecrets.includes(secret.name)}
              />
            ))}
            {customSecrets.map((name) => (
              <SecretRow
                key={name}
                name={name}
                label="Custom secret"
                configured={true}
              />
            ))}
          </div>
          <AddSecretForm existingNames={configuredSecrets} />
        </Panel>
      )}
    </div>
  );
}
