import { AgentsSection } from "../features/agents/AgentsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./AgentsPage.module.css";

export function AgentsPage() {
  useBreadcrumbs([], "Agents");
  return (
    <div className={styles.page}>
      <AgentsSection />
    </div>
  );
}
