import { Panel } from "@metis/ui";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const { user } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : "Unknown";

  return (
    <div className={styles.page}>
      <h1 className={styles.title}>Settings</h1>
      <Panel header={<span className={styles.sectionTitle}>Account</span>}>
        <div className={styles.field}>
          <span className={styles.label}>User</span>
          <span className={styles.value}>{displayName}</span>
        </div>
      </Panel>
      <Panel header={<span className={styles.sectionTitle}>Preferences</span>}>
        <p className={styles.placeholder}>Settings coming soon.</p>
      </Panel>
    </div>
  );
}
