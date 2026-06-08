import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  horizontalListSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
} from "@dnd-kit/sortable";
import { Avatar, Icons, TypeChip } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  ListProjectsResponse,
  ProjectId,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { ProjectChip } from "../../projects/ProjectChip";
import { ProjectSettingsModal } from "../../projects/ProjectSettingsModal";
import { ProjectCreateModal } from "../../projects/ProjectCreateModal";
import { StatusSettingsModal } from "../../projects/StatusSettingsModal";
import { useProjects, useProjectStatuses } from "../../projects/useProjects";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticUpsert,
} from "../../projects/projectCache";
import { apiClient } from "../../../api/client";
import { useToast } from "../../toast/useToast";
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
  const [settingsProjectId, setSettingsProjectId] = useState<ProjectId | null>(null);

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

  // Real-ProjectRecord lookup by project_id for the gear button. The
  // synthesized default project (project_id === null) intentionally has
  // no entry — the gear is suppressed there because there's no record to
  // mutate via apiClient.updateProject.
  const projectRecordById = useMemo(() => {
    const map = new Map<string, ProjectRecord>();
    for (const rec of allProjects ?? []) {
      map.set(rec.project_id, rec);
    }
    return map;
  }, [allProjects]);

  // Track the gear target by projectId, not by the ProjectRecord itself.
  // The modal stays open across Move clicks; we need it to re-render
  // against the freshest ProjectRecord (post optimistic-update) rather
  // than a stale snapshot captured at click time.
  const [gearTarget, setGearTarget] = useState<{
    projectId: string;
    statusKey: string;
    issueCount: number;
  } | null>(null);

  // "+ Add status" target: tracked by projectId for the same freshness
  // reason as gearTarget — the new-status modal needs to see the live
  // statuses array when computing the next default colour.
  const [newStatusProjectId, setNewStatusProjectId] = useState<string | null>(
    null,
  );
  const [newProjectOpen, setNewProjectOpen] = useState(false);

  const handleCardClick = (id: string) => {
    const params = new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    });
    navigate(`/issues/${id}?${params.toString()}`);
  };

  const settingsProject: ProjectRecord | null = useMemo(() => {
    if (!settingsProjectId) return null;
    return (
      (allProjects ?? []).find((p) => p.project_id === settingsProjectId) ?? null
    );
  }, [allProjects, settingsProjectId]);

  const handleGearClick = (
    projectRecord: ProjectRecord,
    statusKey: string,
    cell: BoardCellQuery | undefined,
  ) => {
    // The loaded `cell.issues.length` only reflects fetched pages. If
    // there's another page on the server, treat the column as non-empty
    // for the delete-safety check by passing a positive sentinel — we
    // can't know how many issues live beyond the loaded window.
    const loaded = cell?.issues.length ?? 0;
    const issueCount = cell?.hasNextPage ? Math.max(loaded, 1) : loaded;
    setGearTarget({
      projectId: projectRecord.project_id,
      statusKey,
      issueCount,
    });
  };

  const gearProjectRecord = gearTarget
    ? projectRecordById.get(gearTarget.projectId) ?? null
    : null;

  const newStatusProjectRecord = newStatusProjectId
    ? projectRecordById.get(newStatusProjectId) ?? null
    : null;

  const showNewProjectRow = !baseFilters.project_id;

  return (
    <div className={styles.kanban}>
      {projects.map((project) => {
        const perStatus = cells.get(project.project_id);
        const projectIssueCount = project.statuses.reduce((acc, s) => {
          const cell = perStatus?.get(s.key);
          return acc + (cell?.issues.length ?? 0);
        }, 0);
        const projectRecord =
          project.project_id !== null
            ? projectRecordById.get(project.project_id) ?? null
            : null;
        return (
          <ProjectSection
            key={project.project_id ?? "__default__"}
            project={project}
            projectRecord={projectRecord}
            perStatus={perStatus}
            projectIssueCount={projectIssueCount}
            childStatusMap={childStatusMap}
            onCardClick={handleCardClick}
            onOpenSettings={setSettingsProjectId}
            onGearClick={handleGearClick}
            onAddStatus={setNewStatusProjectId}
          />
        );
      })}
      {showNewProjectRow && (
        <button
          type="button"
          className={styles.newProjectGhost}
          onClick={() => setNewProjectOpen(true)}
          data-testid="board-new-project"
        >
          + New project
        </button>
      )}
      {settingsProject && (
        <ProjectSettingsModal
          open
          onClose={() => setSettingsProjectId(null)}
          project={settingsProject}
        />
      )}
      {gearTarget && gearProjectRecord && (
        <StatusSettingsModal
          open={true}
          onClose={() => setGearTarget(null)}
          projectRecord={gearProjectRecord}
          statusKey={gearTarget.statusKey}
          issueCount={gearTarget.issueCount}
        />
      )}
      {newStatusProjectRecord && (
        <StatusSettingsModal
          open={true}
          mode="new"
          onClose={() => setNewStatusProjectId(null)}
          projectRecord={newStatusProjectRecord}
        />
      )}
      {showNewProjectRow && newProjectOpen && (
        <ProjectCreateModal
          open
          onClose={() => setNewProjectOpen(false)}
        />
      )}
    </div>
  );
}

interface ProjectSectionProps {
  project: BoardProjectDescriptor;
  projectRecord: ProjectRecord | null;
  perStatus: Map<string, BoardCellQuery> | undefined;
  projectIssueCount: number;
  childStatusMap: Map<string, ChildStatus[]>;
  onCardClick: (id: string) => void;
  onOpenSettings: (id: ProjectId) => void;
  onGearClick: (
    projectRecord: ProjectRecord,
    statusKey: string,
    cell: BoardCellQuery | undefined,
  ) => void;
  onAddStatus: (projectId: string) => void;
}

function ProjectSection({
  project,
  projectRecord,
  perStatus,
  projectIssueCount,
  childStatusMap,
  onCardClick,
  onOpenSettings,
  onGearClick,
  onAddStatus,
}: ProjectSectionProps) {
  // Hook calls must be unconditional. The default-project section
  // (projectRecord === null) skips DnD entirely, but the mutation/sensor
  // hooks below are cheap to declare and harmless when unused.
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const reorderMutation = useMutation({
    mutationFn: async (nextStatuses: StatusDefinition[]) => {
      if (!projectRecord) {
        throw new Error("Cannot reorder columns on the default project");
      }
      return apiClient.updateProject(projectRecord.project_id, {
        project: { ...projectRecord.project, statuses: nextStatuses },
      });
    },
    onMutate: async (nextStatuses) => {
      if (!projectRecord) return { previous: undefined };
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const nextProject = {
          ...projectRecord.project,
          statuses: nextStatuses,
        };
        const next: ListProjectsResponse = {
          projects: applyOptimisticUpsert(
            previous.projects,
            projectRecord.project_id,
            nextProject,
          ),
        };
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, next);
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(PROJECTS_QUERY_KEY, context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to reorder columns",
        "error",
      );
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
      queryClient.invalidateQueries({ queryKey: ["project-statuses"] });
    },
  });

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const sortableIds = useMemo(
    () => project.statuses.map((s) => s.key),
    [project.statuses],
  );

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      if (!projectRecord) return;
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const statuses = projectRecord.project.statuses;
      const oldIdx = statuses.findIndex((s) => s.key === active.id);
      const newIdx = statuses.findIndex((s) => s.key === over.id);
      if (oldIdx < 0 || newIdx < 0) return;
      const next = arrayMove(statuses, oldIdx, newIdx);
      reorderMutation.mutate(next);
    },
    [projectRecord, reorderMutation],
  );

  const columns = project.statuses.map((status) => {
    const cell = perStatus?.get(status.key);
    if (projectRecord) {
      return (
        <SortableBoardColumn
          key={status.key}
          project={project}
          projectRecord={projectRecord}
          status={status}
          cell={cell}
          childStatusMap={childStatusMap}
          onCardClick={onCardClick}
          onGearClick={onGearClick}
        />
      );
    }
    return (
      <BoardColumn
        key={status.key}
        project={project}
        projectRecord={null}
        status={status}
        cell={cell}
        childStatusMap={childStatusMap}
        onCardClick={onCardClick}
        onGearClick={onGearClick}
      />
    );
  });

  const columnsRow = (
    <div className={styles.projectColumns}>
      {columns}
      {projectRecord && (
        <button
          type="button"
          className={styles.colGhost}
          onClick={() => onAddStatus(projectRecord.project_id)}
          aria-label={`Add status to ${project.name}`}
          data-testid={`board-col-add-${project.key}`}
        >
          + Add status
        </button>
      )}
    </div>
  );

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
        <div
          className={styles.projectBarRight}
          data-testid={`board-project-actions-${project.key}`}
        >
          {project.project_id !== null && (
            <button
              type="button"
              className={styles.projectSettingsButton}
              onClick={() => onOpenSettings(project.project_id as ProjectId)}
              title="Project settings"
              aria-label={`Project settings for ${project.name}`}
              data-testid={`board-project-settings-${project.key}`}
            >
              <Icons.IconSettings size={14} />
            </button>
          )}
        </div>
      </div>
      {projectRecord ? (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          onDragEnd={handleDragEnd}
        >
          <SortableContext
            items={sortableIds}
            strategy={horizontalListSortingStrategy}
          >
            {columnsRow}
          </SortableContext>
        </DndContext>
      ) : (
        columnsRow
      )}
    </section>
  );
}

interface BoardColumnProps {
  project: BoardProjectDescriptor;
  projectRecord: ProjectRecord | null;
  status: StatusDefinition;
  cell: BoardCellQuery | undefined;
  childStatusMap: Map<string, ChildStatus[]>;
  onCardClick: (id: string) => void;
  onGearClick: (
    projectRecord: ProjectRecord,
    statusKey: string,
    cell: BoardCellQuery | undefined,
  ) => void;
}

interface SortableHandleProps {
  setNodeRef?: (node: HTMLElement | null) => void;
  style?: React.CSSProperties;
  isDragging?: boolean;
  dragHandleProps?: React.HTMLAttributes<HTMLElement>;
}

function transformToCss(
  transform: { x: number; y: number; scaleX: number; scaleY: number } | null,
): string | undefined {
  if (!transform) return undefined;
  return `translate3d(${transform.x}px, ${transform.y}px, 0) scaleX(${transform.scaleX}) scaleY(${transform.scaleY})`;
}

function SortableBoardColumn(props: BoardColumnProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
    isOver,
  } = useSortable({ id: props.status.key });
  const style: React.CSSProperties = {
    transform: transformToCss(transform),
    transition: transition ?? undefined,
    opacity: isDragging ? 0.6 : undefined,
  };
  return (
    <BoardColumn
      {...props}
      setNodeRef={setNodeRef}
      style={style}
      isDragging={isDragging}
      isOver={isOver}
      dragHandleProps={{ ...attributes, ...listeners }}
    />
  );
}

function BoardColumn({
  project,
  projectRecord,
  status,
  cell,
  childStatusMap,
  onCardClick,
  onGearClick,
  setNodeRef,
  style,
  isDragging,
  isOver,
  dragHandleProps,
}: BoardColumnProps & SortableHandleProps & { isOver?: boolean }) {
  const colIssues = cell?.issues ?? [];
  const showInitialLoading = (cell?.isLoading ?? false) && colIssues.length === 0;
  const isDefaultStatus = status.key === project.default_status_key;
  const assignTo = status.on_enter?.assign_to ?? null;
  const interactiveLabel = status.interactive === true ? "interactive" : "auto";
  const colClasses = [styles.col];
  if (isDragging) colClasses.push(styles.colDragging);
  if (isOver && !isDragging) colClasses.push(styles.colDropOver);
  return (
    <div
      ref={setNodeRef}
      style={style}
      className={colClasses.join(" ")}
      data-testid={`board-col-${project.key}-${status.key}`}
    >
      <div
        className={
          dragHandleProps
            ? `${styles.colHead} ${styles.colHeadDraggable}`
            : styles.colHead
        }
        data-testid={`board-col-head-${project.key}-${status.key}`}
        {...(dragHandleProps ?? {})}
      >
        <StatusChip definition={status} />
        {isDefaultStatus && <span className={styles.defaultChip}>DEFAULT</span>}
        <span className={styles.colCount}>{colIssues.length}</span>
        {projectRecord && (
          <button
            type="button"
            className={styles.colGear}
            onClick={() => onGearClick(projectRecord, status.key, cell)}
            aria-label={`Settings for ${status.label || status.key}`}
            title="Status settings"
            data-testid={`board-col-gear-${project.key}-${status.key}`}
          >
            ⚙
          </button>
        )}
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
