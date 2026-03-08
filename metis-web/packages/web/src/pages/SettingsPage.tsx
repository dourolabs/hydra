import { RepositoriesSection } from "../features/repositories/RepositoriesSection";
import { AgentsSection } from "../features/agents/AgentsSection";
import { SecretsSection } from "../features/secrets/SecretsSection";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  return (
    <div className={styles.page}>
      <RepositoriesSection />
      <AgentsSection />
      <SecretsSection />
    </div>
  );
}
