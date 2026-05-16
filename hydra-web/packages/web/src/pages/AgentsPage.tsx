import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useAgents } from "../hooks/useAgents";
import { AgentsSection } from "../features/agents/AgentsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./AgentsPage.module.css";

export function AgentsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Agents");
  const [createOpen, setCreateOpen] = useState(false);
  const { data: agents } = useAgents();
  const count = agents?.length ?? 0;
  const label = count === 1 ? "1 AGENT" : `${count} AGENTS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>SYSTEM · {label}</span>
          <h1 className={styles.pageTitle}>Agents</h1>
        </div>
        <span className={styles.headSpacer} />
        <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
          <Icons.IconPlus />
          Add agent
        </Button>
      </div>

      <div className={styles.body}>
        <AgentsSection
          createOpen={createOpen}
          onCreateOpenChange={setCreateOpen}
        />
      </div>
    </div>
  );
}
