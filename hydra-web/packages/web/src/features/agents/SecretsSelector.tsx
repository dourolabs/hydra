import { useUsername } from "../auth/useUsername";
import { useSecrets } from "../secrets/useSecrets";
import styles from "./AgentsSection.module.css";

interface SecretsSelectorProps {
  selected: string[];
  onChange: (secrets: string[]) => void;
}

export function SecretsSelector({ selected, onChange }: SecretsSelectorProps) {
  const username = useUsername();
  const { data, isLoading } = useSecrets(username);
  const availableSecrets = data?.secrets ?? [];

  const toggle = (name: string) => {
    if (selected.includes(name)) {
      onChange(selected.filter((s) => s !== name));
    } else {
      onChange([...selected, name]);
    }
  };

  if (isLoading) {
    return (
      <div className={styles.secretsField}>
        <span className={styles.secretsFieldLabel}>Secrets</span>
        <span className={styles.dimText}>Loading secrets...</span>
      </div>
    );
  }

  if (availableSecrets.length === 0) {
    return (
      <div className={styles.secretsField}>
        <span className={styles.secretsFieldLabel}>Secrets</span>
        <span className={styles.dimText}>
          No secrets configured. Add secrets in the Secrets section above.
        </span>
      </div>
    );
  }

  return (
    <div className={styles.secretsField}>
      <span className={styles.secretsFieldLabel}>Secrets</span>
      <div className={styles.secretsCheckboxList}>
        {availableSecrets.map((name) => (
          <label key={name} className={styles.checkboxLabel}>
            <input
              type="checkbox"
              checked={selected.includes(name)}
              onChange={() => toggle(name)}
            />
            {name}
          </label>
        ))}
      </div>
    </div>
  );
}
