import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useUsername } from "../features/auth/useUsername";
import { useSecrets } from "../features/secrets/useSecrets";
import { SecretsSection } from "../features/secrets/SecretsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./SecretsPage.module.css";

export function SecretsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Secrets");
  const username = useUsername();
  const [adding, setAdding] = useState(false);
  const { data } = useSecrets(username);
  const count = data?.secrets.length ?? 0;
  const label = count === 1 ? "1 SECRET" : `${count} SECRETS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>SYSTEM · {label}</span>
          <h1 className={styles.pageTitle}>Secrets</h1>
        </div>
        <span className={styles.headSpacer} />
        <Button variant="primary" size="sm" onClick={() => setAdding(true)}>
          <Icons.IconPlus />
          Add secret
        </Button>
      </div>

      <div className={styles.body}>
        <SecretsSection adding={adding} onAddingChange={setAdding} />
      </div>
    </div>
  );
}
