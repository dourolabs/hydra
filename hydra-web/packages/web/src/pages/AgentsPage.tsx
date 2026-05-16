import { AgentsSection } from "../features/agents/AgentsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./AgentsPage.module.css";

export function AgentsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Agents");
  return (
    <div className={styles.page}>
      <AgentsSection />
    </div>
  );
}
