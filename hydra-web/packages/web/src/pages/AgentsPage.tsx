import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useAgents } from "../hooks/useAgents";
import { AgentsSection } from "../features/agents/AgentsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import { FloatingActionButton } from "../layout/FloatingActionButton";
import styles from "./AgentsPage.module.css";

export function AgentsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Agents");
  const [createOpen, setCreateOpen] = useState(false);
  const { data: agents } = useAgents();
  const count = agents?.length ?? 0;
  const label = count === 1 ? "1 AGENT" : `${count} AGENTS`;

  return (
    <div className={styles.page}>
      <PageHead
        eyebrow={`SYSTEM · ${label}`}
        title="Agents"
        actions={
          <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
            <Icons.IconPlus />
            Add agent
          </Button>
        }
      />

      <div className={styles.body}>
        <AgentsSection
          createOpen={createOpen}
          onCreateOpenChange={setCreateOpen}
        />
      </div>
      <FloatingActionButton
        icon={<Icons.IconPlus size={24} />}
        label="Add agent"
        onClick={() => setCreateOpen(true)}
        testId="agents-fab"
      />
    </div>
  );
}
