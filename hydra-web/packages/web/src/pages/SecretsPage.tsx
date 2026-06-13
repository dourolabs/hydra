import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useUsername } from "../features/auth/useUsername";
import { useSecrets } from "../features/secrets/useSecrets";
import { SecretsSection } from "../features/secrets/SecretsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import { FloatingActionButton } from "../layout/FloatingActionButton";
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
      <PageHead
        eyebrow={`SYSTEM · ${label}`}
        title="Secrets"
        actions={
          <Button variant="primary" size="sm" onClick={() => setAdding(true)}>
            <Icons.IconPlus />
            Add secret
          </Button>
        }
      />

      <div className={styles.body}>
        <SecretsSection adding={adding} onAddingChange={setAdding} />
      </div>
      <FloatingActionButton
        icon={<Icons.IconPlus size={24} />}
        label="Add secret"
        onClick={() => setAdding(true)}
        testId="secrets-fab"
      />
    </div>
  );
}
