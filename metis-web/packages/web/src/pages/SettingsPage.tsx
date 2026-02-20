import { Panel } from "@metis/ui";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  return (
    <div className={styles.page}>
      <Panel header={<span className={styles.header}>Settings</span>}>
        <p className={styles.placeholder}>Settings page coming soon.</p>
      </Panel>
    </div>
  );
}
