import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useTriggers } from "../features/triggers/useTriggers";
import { TriggersSection } from "../features/triggers/TriggersSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import { FloatingActionButton } from "../layout/FloatingActionButton";
import styles from "./TriggersListPage.module.css";

export function TriggersListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Triggers");
  const [createOpen, setCreateOpen] = useState(false);
  const { data: triggers } = useTriggers();
  const count = triggers?.length ?? 0;
  const label = count === 1 ? "1 TRIGGER" : `${count} TRIGGERS`;

  return (
    <div className={styles.page}>
      <PageHead
        eyebrow={`WORKSPACE · ${label}`}
        title="Triggers"
        actions={
          <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
            <Icons.IconPlus />
            Add trigger
          </Button>
        }
      />

      <div className={styles.body}>
        <TriggersSection
          createOpen={createOpen}
          onCreateOpenChange={setCreateOpen}
        />
      </div>
      <FloatingActionButton
        icon={<Icons.IconPlus size={24} />}
        label="Add trigger"
        onClick={() => setCreateOpen(true)}
        testId="triggers-fab"
      />
    </div>
  );
}
