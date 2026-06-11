import { useState } from "react";
import { Button, Icons } from "@hydra/ui";
import { IssuesBoard } from "../features/issues/view/IssuesBoard";
import { ProjectCreateModal } from "../features/projects/ProjectCreateModal";
import { useProjects } from "../features/projects/useProjects";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import styles from "./ProjectsListPage.module.css";

export function ProjectsListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Projects");
  const [createOpen, setCreateOpen] = useState(false);
  const { data: projects } = useProjects();
  const count = projects?.length ?? 0;
  const label = count === 1 ? "1 PROJECT" : `${count} PROJECTS`;

  return (
    <div className={styles.page}>
      <PageHead
        eyebrow={`WORKSPACE · ${label}`}
        title="Projects"
        actions={
          <Button
            variant="primary"
            size="sm"
            onClick={() => setCreateOpen(true)}
            data-testid="projects-list-add"
          >
            <Icons.IconPlus />
            Add project
          </Button>
        }
      />

      <div className={styles.body}>
        <IssuesBoard
          baseFilters={{}}
          filterRootId={null}
          hideIssues
        />
      </div>

      <ProjectCreateModal open={createOpen} onClose={() => setCreateOpen(false)} />
    </div>
  );
}
