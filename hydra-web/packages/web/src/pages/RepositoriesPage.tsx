import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { useRepositories } from "../hooks/useRepositories";
import { RepositoriesSection } from "../features/repositories/RepositoriesSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import styles from "./RepositoriesPage.module.css";

export function RepositoriesPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Repositories");
  const [createOpen, setCreateOpen] = useState(false);
  const { data: repositories } = useRepositories();
  const count = repositories?.length ?? 0;
  const label = count === 1 ? "1 REPO" : `${count} REPOS`;

  return (
    <div className={styles.page}>
      <PageHead
        eyebrow={`SYSTEM · ${label}`}
        title="Repositories"
        actions={
          <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
            <Icons.IconPlus />
            Add repository
          </Button>
        }
      />

      <div className={styles.body}>
        <RepositoriesSection
          createOpen={createOpen}
          onCreateOpenChange={setCreateOpen}
        />
      </div>
    </div>
  );
}
