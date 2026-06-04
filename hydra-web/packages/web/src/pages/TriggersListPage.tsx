import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useTriggers } from "../features/triggers/useTriggers";
import { TriggersSection } from "../features/triggers/TriggersSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./TriggersListPage.module.css";

export function TriggersListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Triggers");
  const [createOpen, setCreateOpen] = useState(false);
  const { data: triggers } = useTriggers();
  const count = triggers?.length ?? 0;
  const label = count === 1 ? "1 TRIGGER" : `${count} TRIGGERS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>WORKSPACE · {label}</span>
          <h1 className={styles.pageTitle}>Triggers</h1>
        </div>
        <span className={styles.headSpacer} />
        <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
          <Icons.IconPlus />
          Add trigger
        </Button>
      </div>

      <div className={styles.body}>
        <TriggersSection
          createOpen={createOpen}
          onCreateOpenChange={setCreateOpen}
        />
      </div>
    </div>
  );
}
