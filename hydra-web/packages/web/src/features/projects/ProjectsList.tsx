import { Link } from "react-router-dom";
import type { ProjectRecord } from "@hydra/api";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { useProjects } from "./useProjects";
import { StatusChip } from "./StatusChip";
import styles from "./ProjectsList.module.css";

export function ProjectsList() {
  const { data: projects, isLoading, error, refetch } = useProjects();

  if (isLoading) return <LoadingState />;
  if (error) {
    return (
      <ErrorState
        message={`Failed to load projects: ${(error as Error).message}`}
        onRetry={() => refetch()}
      />
    );
  }

  if (!projects || projects.length === 0) {
    return (
      <EmptyState message="No projects yet. Create one to define a custom status pipeline." />
    );
  }

  return (
    <div className={styles.list} data-testid="projects-list">
      {projects.map((record: ProjectRecord) => {
        const project = record.project;
        return (
          <Link
            key={record.project_id}
            to={`/projects/${project.key}`}
            className={styles.row}
            data-testid={`projects-list-row-${project.key}`}
          >
            <span className={styles.name}>{project.name}</span>
            <span className={styles.key}>{project.key}</span>
            <span className={styles.statuses}>
              {project.statuses.map((s) => (
                <StatusChip key={s.key} definition={s} />
              ))}
            </span>
          </Link>
        );
      })}
    </div>
  );
}
