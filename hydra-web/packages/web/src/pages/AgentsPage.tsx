import { AgentsSection } from "../features/agents/AgentsSection";
import styles from "./AgentsPage.module.css";

export function AgentsPage() {
  return (
    <div className={styles.page}>
      <AgentsSection />
    </div>
  );
}
