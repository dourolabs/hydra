import { useMemo } from "react";
import { useParams } from "react-router-dom";
import { useProjects } from "../features/projects/useProjects";
import { ProjectEditor } from "../features/projects/ProjectEditor";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { useUsername } from "../features/auth/useUsername";
import styles from "./TriggersListPage.module.css";

export function ProjectDetailPage() {
  const { projectKey } = useParams<{ projectKey: string }>();
  const { data: projects, isLoading, error, refetch } = useProjects();
  const username = useUsername() ?? "unknown";

  const record = useMemo(
    () => projects?.find((p) => p.project.key === projectKey),
    [projects, projectKey],
  );

  useBreadcrumbs(
    [{ label: "Workspace", to: "/" }, { label: "Projects", to: "/projects" }],
    record?.project.name ?? projectKey ?? "Project",
  );

  if (isLoading) return <LoadingState />;
  if (error) {
    return (
      <ErrorState
        message={`Failed to load projects: ${(error as Error).message}`}
        onRetry={() => refetch()}
      />
    );
  }
  if (!record) {
    return <EmptyState message={`Project '${projectKey}' not found.`} />;
  }

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>WORKSPACE · PROJECT</span>
          <h1 className={styles.pageTitle}>{record.project.name}</h1>
        </div>
        <span className={styles.headSpacer} />
      </div>

      <div className={styles.body}>
        <ProjectEditor
          projectId={record.project_id}
          initial={record.project}
          creator={record.project.creator ?? username}
        />
      </div>
    </div>
  );
}
