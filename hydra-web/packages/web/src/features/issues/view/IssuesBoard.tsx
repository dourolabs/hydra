import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, TypeChip } from "@hydra/ui";
import type { IssueSummaryRecord, StatusDefinition } from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { ProjectChip } from "../../projects/ProjectChip";
import { useProjects, useProjectStatuses } from "../../projects/useProjects";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import {
  useBoardIssuesByProject,
  type BoardCellQuery,
  type BoardProjectDescriptor,
  type IssueFilters,
} from "../usePaginatedIssues";
import { usePageIssueTrees } from "../../dashboard/usePageIssueTrees";
import { AgoTime } from "../../../components/Runtime/Runtime";
import styles from "./IssuesBoard.module.css";

interface IssuesBoardProps {
  baseFilters: IssueFilters;
  username: string;
  filterRootId: string | null;
}

function progressFraction(children: ChildStatus[] | undefined): number {
  if (!children || children.length === 0) return 0;
  const total = children.length;
  const done = children.filter(
    (c) => c.status === "closed" || c.status === "issue-closed",
  ).length;
  return Math.round((done / total) * 100);
}

const DEFAULT_PROJECT_KEY = "default";
const DEFAULT_PROJECT_NAME = "Default";

export function IssuesBoard({ baseFilters, username, filterRootId }: IssuesBoardProps) {
  const navigate = useNavigate();
  const { data: allProjects } = useProjects();
  const { data: defaultStatusesResponse } = useProjectStatuses(null);

  // Build the section list: synthesized default project first, then any
  // user-defined projects (filtered by `baseFilters.project_id` if the page
  // is scoped to a single project). Once [[i-vstbajos]] lands and the
  // default project becomes a real ProjectRecord, the synthesized branch
  // becomes dead and can be removed.
  const projects: BoardProjectDescriptor[] = useMemo(() => {
    const out: BoardProjectDescriptor[] = [];
    const defaultStatuses = defaultStatusesResponse?.statuses ?? [];
    const defaultStatusKey = defaultStatusesResponse?.default_status_key ?? "open";
    const realProjects = (allProjects ?? []).filter(
      (record) => !record.project.deleted,
    );

    if (baseFilters.project_id) {
      const match = realProjects.find(
        (p) => p.project_id === baseFilters.project_id,
      );
      if (match) {
        out.push({
          project_id: match.project_id,
          key: match.project.key,
          name: match.project.name,
          statuses: match.project.statuses,
          default_status_key: match.project.default_status_key,
        });
      }
      return out;
    }

    if (defaultStatuses.length > 0) {
      out.push({
        project_id: null,
        key: DEFAULT_PROJECT_KEY,
        name: DEFAULT_PROJECT_NAME,
        statuses: defaultStatuses,
        default_status_key: defaultStatusKey,
      });
    }
    for (const record of realProjects) {
      out.push({
        project_id: record.project_id,
        key: record.project.key,
        name: record.project.name,
        statuses: record.project.statuses,
        default_status_key: record.project.default_status_key,
      });
    }
    return out;
  }, [allProjects, defaultStatusesResponse, baseFilters.project_id]);

  const cells = useBoardIssuesByProject(baseFilters, projects);

  // Union of all visible issues for tree resolution.
  const boardIssuesUnion = useMemo(() => {
    const seen = new Set<string>();
    const out: IssueSummaryRecord[] = [];
    for (const project of projects) {
      const perStatus = cells.get(project.project_id);
      if (!perStatus) continue;
      for (const status of project.statuses) {
        const cell = perStatus.get(status.key);
        if (!cell) continue;
        for (const rec of cell.issues) {
          if (seen.has(rec.issue_id)) continue;
          seen.add(rec.issue_id);
          out.push(rec);
        }
      }
    }
    return out;
  }, [projects, cells]);

  const { childStatusMap } = usePageIssueTrees(boardIssuesUnion, username);

  const handleCardClick = (id: string) => {
    const params = new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    });
    navigate(`/issues/${id}?${params.toString()}`);
  };

  return (
    <div className={styles.kanban}>
      {projects.map((project) => {
        const perStatus = cells.get(project.project_id);
        const projectIssueCount = project.statuses.reduce((acc, s) => {
          const cell = perStatus?.get(s.key);
          return acc + (cell?.issues.length ?? 0);
        }, 0);
        return (
          <ProjectSection
            key={project.project_id ?? "__default__"}
            project={project}
            perStatus={perStatus}
            projectIssueCount={projectIssueCount}
            childStatusMap={childStatusMap}
            onCardClick={handleCardClick}
          />
        );
      })}
    </div>
  );
}

interface ProjectSectionProps {
  project: BoardProjectDescriptor;
  perStatus: Map<string, BoardCellQuery> | undefined;
  projectIssueCount: number;
  childStatusMap: Map<string, ChildStatus[]>;
  onCardClick: (id: string) => void;
}

function ProjectSection({
  project,
  perStatus,
  projectIssueCount,
  childStatusMap,
  onCardClick,
}: ProjectSectionProps) {
  return (
    <section
      className={styles.projectGroup}
      data-testid={`board-project-${project.key}`}
    >
      <div className={styles.projectBar}>
        <div className={styles.projectBarLeft}>
          <ProjectChip
            projectKey={project.key}
            name={project.name}
            data-testid={`board-project-chip-${project.key}`}
          />
          <span className={styles.projectMeta}>
            {projectIssueCount} {projectIssueCount === 1 ? "issue" : "issues"}
          </span>
          <span className={styles.projectMeta}>
            {project.statuses.length}{" "}
            {project.statuses.length === 1 ? "status" : "statuses"}
          </span>
        </div>
        {/* Placeholder slot for the future ⚙ settings gear / Add status
            button (deferred to follow-up PR). Empty div reserves the
            layout slot without rendering dead UI. */}
        <div
          className={styles.projectBarRight}
          data-testid={`board-project-actions-${project.key}`}
        />
      </div>
      <div className={styles.projectColumns}>
        {project.statuses.map((status) => {
          const cell = perStatus?.get(status.key);
          return (
            <BoardColumn
              key={status.key}
              project={project}
              status={status}
              cell={cell}
              childStatusMap={childStatusMap}
              onCardClick={onCardClick}
            />
          );
        })}
      </div>
    </section>
  );
}

interface BoardColumnProps {
  project: BoardProjectDescriptor;
  status: StatusDefinition;
  cell: BoardCellQuery | undefined;
  childStatusMap: Map<string, ChildStatus[]>;
  onCardClick: (id: string) => void;
}

function BoardColumn({
  project,
  status,
  cell,
  childStatusMap,
  onCardClick,
}: BoardColumnProps) {
  const colIssues = cell?.issues ?? [];
  const showInitialLoading = (cell?.isLoading ?? false) && colIssues.length === 0;
  const isDefaultStatus = status.key === project.default_status_key;
  const assignTo = status.on_enter?.assign_to ?? null;
  const interactiveLabel = status.interactive === true ? "interactive" : "auto";
  return (
    <div className={styles.col} data-testid={`board-col-${project.key}-${status.key}`}>
      <div className={styles.colHead}>
        <StatusChip definition={status} />
        {isDefaultStatus && <span className={styles.defaultChip}>DEFAULT</span>}
        <span className={styles.colCount}>{colIssues.length}</span>
      </div>
      <div
        className={styles.colSubHead}
        data-testid={`board-col-subhead-${project.key}-${status.key}`}
      >
        {assignTo && (
          <span className={styles.subHeadAssignee}>
            <Avatar
              name={principalDisplayName(assignTo)}
              kind={principalAvatarKind(assignTo)}
              size="sm"
            />
            <span className={styles.subHeadName}>
              {principalDisplayName(assignTo)}
            </span>
          </span>
        )}
        <span
          className={styles.modeBadge}
          data-testid={`board-col-mode-${project.key}-${status.key}`}
        >
          {interactiveLabel}
        </span>
      </div>
      <div className={styles.colBody}>
        {showInitialLoading && (
          <div className={styles.colEmpty}>Loading…</div>
        )}
        {!showInitialLoading && colIssues.length === 0 && (
          <div className={styles.colEmpty}>No issues</div>
        )}
        {colIssues.map((rec) => {
          const issue = rec.issue;
          const id = rec.issue_id;
          const children = childStatusMap.get(id);
          const pct = progressFraction(children);

          return (
            <div
              key={id}
              className={styles.card}
              onClick={() => onCardClick(id)}
            >
              {issue.type && issue.type !== "unknown" && (
                <div className={styles.cardHead}>
                  <TypeChip type={issue.type} />
                </div>
              )}
              <div className={styles.cardTitle}>{issue.title || "(untitled)"}</div>
              <div className={styles.cardFoot}>
                {issue.assignee && (
                  <Avatar
                    name={principalDisplayName(issue.assignee)}
                    kind={principalAvatarKind(issue.assignee)}
                    size="md"
                  />
                )}
                <AgoTime iso={rec.timestamp} />
                <span className={styles.cardFootSpacer} />
                {children && children.length > 0 && (
                  <div className={styles.progress} title={`${pct}%`}>
                    <span style={{ width: `${pct}%` }} />
                  </div>
                )}
              </div>
            </div>
          );
        })}
        {cell?.hasNextPage && (
          <div className={styles.colLoadMore}>
            <button
              type="button"
              className={styles.colLoadMoreButton}
              onClick={cell.fetchNextPage}
              disabled={cell.isFetchingNextPage}
              data-testid={`issues-board-load-more-${project.key}-${status.key}`}
            >
              {cell.isFetchingNextPage ? "Loading…" : "Load more"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
