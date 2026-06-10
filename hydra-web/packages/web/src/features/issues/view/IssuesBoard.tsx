import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MeasuringStrategy,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
  type Modifier,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  horizontalListSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { Avatar, FlowPill, Icons, TypeChip } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  ListIssuesResponse,
  ListProjectsResponse,
  Project,
  ProjectId,
  ProjectRecord,
  StatusDefinition,
  StatusKey,
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
import { useProjects } from "../../projects/useProjects";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticUpsert,
} from "../../projects/projectCache";
import { apiClient } from "../../../api/client";
import { useIssueCreateModal } from "../../dashboard/useIssueCreateModal";
import { useToast } from "../../toast/useToast";
import { computeFlowPillState, type IssueNeighborhood } from "../flowPill";
import {
  useBoardIssuesByProject,
  type BoardCellQuery,
  type BoardProjectDescriptor,
  type IssueFilters,
} from "../usePaginatedIssues";
import { usePageIssueTrees } from "../../dashboard/usePageIssueTrees";
import { AgoTime } from "../../../components/Runtime/Runtime";
import { useProjectCollapseState } from "./useProjectCollapseState";
import { RestoreIssueButton } from "../RestoreIssueButton";
import styles from "./IssuesBoard.module.css";

interface IssuesBoardProps {
  baseFilters: IssueFilters;
  filterRootId: string | null;
  // Projects-tab variant: render the same board chrome (project bars,
  // status columns, ghost rows) but skip per-cell issue fetches and
  // suppress everything that's about issues — counts, "No issues"
  // placeholders, the project bar's "N issues" pill. The card bodies stay
  // empty by virtue of the fetch being disabled.
  hideIssues?: boolean;
}

// Step used at the ends of the list when there is no opposite neighbor to
// midpoint against. Large enough to leave headroom for many subsequent
// inserts before float precision forces a renumber.
const PROJECT_PRIORITY_STEP = 1024;

// Pick a new `priority` for a project that just landed between `left` and
// `right` after a drag-and-drop. Mid-points between neighbors give O(1)
// reorders; ends extend by a fixed step.
function computeReorderPriority(
  left: ProjectRecord | undefined,
  right: ProjectRecord | undefined,
): number {
  if (left && right) {
    return (left.project.priority + right.project.priority) / 2;
  }
  if (right) return right.project.priority - PROJECT_PRIORITY_STEP;
  if (left) return left.project.priority + PROJECT_PRIORITY_STEP;
  return 0;
}

// Keep the DragOverlay header vertically centred on the cursor. Because every
// section collapses to its bar at drag start and we re-measure continuously
// (MeasuringStrategy.Always), the overlay's measured origin shifts up by the
// height collapsed above the grabbed bar — which would otherwise drift the
// preview off the pointer for any bar below the first. Snapping to the cursor
// each frame makes that measured origin irrelevant. (This is the vertical half
// of dnd-kit's snapCenterToCursor; horizontal is left alone so the full-width
// header keeps spanning the board.)
const snapHeaderToCursorY: Modifier = ({
  activatorEvent,
  draggingNodeRect,
  transform,
}) => {
  if (draggingNodeRect && activatorEvent && "clientY" in activatorEvent) {
    const offsetY = (activatorEvent as PointerEvent).clientY - draggingNodeRect.top;
    return { ...transform, y: transform.y + offsetY - draggingNodeRect.height / 2 };
  }
  return transform;
};

function sortProjectsByPriority(list: ProjectRecord[]): ProjectRecord[] {
  return list
    .slice()
    .sort((a, b) => a.project.priority - b.project.priority);
}

// Custom dataTransfer MIME for issue-card drags. Picked up by the column
// drop handler to disambiguate from arbitrary OS-level drags (file drops,
// text selections, etc.) so neither side preventDefaults the wrong drop.
const ISSUE_CARD_DRAG_MIME = "application/x.hydra-issue-card";

interface IssueDragPayload {
  issueId: string;
  sourceProjectId: string;
  sourceStatusKey: string;
}

interface IssueDropTarget {
  projectId: string;
  statusKey: string;
}

type IssueDropHandler = (
  payload: IssueDragPayload,
  target: IssueDropTarget,
) => void;

export function IssuesBoard({
  baseFilters,
  filterRootId,
  hideIssues = false,
}: IssuesBoardProps) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const { data: allProjects } = useProjects();
  const { open: openIssueCreate } = useIssueCreateModal();
  const { isCollapsed, onToggle: onToggleCollapse } = useProjectCollapseState();
  const [settingsProjectId, setSettingsProjectId] = useState<ProjectId | null>(null);

  const projects: BoardProjectDescriptor[] = useMemo(() => {
    const out: BoardProjectDescriptor[] = [];
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
        });
      }
      return out;
    }

    for (const record of realProjects) {
      out.push({
        project_id: record.project_id,
        key: record.project.key,
        name: record.project.name,
        statuses: record.project.statuses,
      });
    }
    return out;
  }, [allProjects, baseFilters.project_id]);

  const cells = useBoardIssuesByProject(baseFilters, projects, !hideIssues);

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

  const { neighborhoodMap } = usePageIssueTrees(
    hideIssues ? [] : boardIssuesUnion,
  );

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

  // The project section currently being dragged. Drives the DragOverlay so
  // the thing following the cursor is a compact fixed-size header rather than
  // the full-height section (which dnd-kit would otherwise stretch to match
  // each neighbor it passes over).
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);

  const handleCardClick = (id: string) => {
    const params = new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    });
    navigate(`/issues/${id}?${params.toString()}`);
  };

  const handleAddIssueClick = useCallback(
    (projectId: string, statusKey: string) => {
      openIssueCreate({ projectId, status: statusKey });
    },
    [openIssueCreate],
  );

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
  // Only mount the project-reorder DnD when there's more than one project
  // to reorder *and* the board isn't scoped to a single project.
  const allowProjectReorder = !baseFilters.project_id && projects.length > 1;

  const projectReorder = useMutation({
    mutationFn: async ({
      projectRecord,
      priority,
    }: {
      projectRecord: ProjectRecord;
      priority: number;
    }) => {
      return apiClient.updateProject(projectRecord.project_id, {
        key: projectRecord.project.key,
        name: projectRecord.project.name,
        prompt_path: projectRecord.project.prompt_path ?? null,
        priority,
      });
    },
    onMutate: async ({ projectRecord, priority }) => {
      await queryClient.cancelQueries({ queryKey: PROJECTS_QUERY_KEY });
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const nextProject: Project = { ...projectRecord.project, priority };
        const upserted = applyOptimisticUpsert(
          previous.projects,
          projectRecord.project_id,
          nextProject,
        );
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, {
          projects: sortProjectsByPriority(upserted),
        });
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(PROJECTS_QUERY_KEY, context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to reorder projects",
        "error",
      );
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["project", response.project_id] });
    },
  });

  // Cross-column / cross-project issue-card drag. The mutation fetches the
  // full Issue (so the truncated `IssueSummary` description / progress
  // doesn't clobber the server-side body) and persists `status` +
  // `project_id` via the existing update-issue endpoint.
  const moveIssue = useMutation({
    mutationFn: async ({
      issueId,
      targetProjectId,
      targetStatusKey,
    }: {
      issueId: string;
      sourceProjectId: string;
      sourceStatusKey: string;
      targetProjectId: string;
      targetStatusKey: StatusKey;
    }) => {
      const record = await apiClient.getIssue(issueId);
      return apiClient.updateIssue(issueId, {
        issue: {
          ...record.issue,
          status: targetStatusKey,
          project_id: targetProjectId,
        },
        session_id: null,
      });
    },
    onMutate: async ({
      issueId,
      sourceProjectId,
      sourceStatusKey,
      targetProjectId,
      targetStatusKey,
    }) => {
      await queryClient.cancelQueries({ queryKey: ["paginatedIssues"] });

      // The `["paginatedIssues", …]` prefix is shared with the table-view
      // `usePaginatedIssues` hook, which is a `useInfiniteQuery` whose `data`
      // is `{ pages, pageParams }` — not the per-cell `ListIssuesResponse[]`
      // produced by `useBoardIssuesByProject`. Filter to the board-cell key
      // shape (`[…, filters, "depth", depth]`) so the optimistic surgery
      // never tries to iterate the infinite-query payload.
      const all = queryClient
        .getQueryCache()
        .findAll({ queryKey: ["paginatedIssues"] })
        .filter((q) => {
          const key = q.queryKey as readonly unknown[];
          return key.length === 4 && key[2] === "depth";
        });

      // Locate the source record so the optimistic insert into the target
      // cell carries the same summary fields as the rendered card.
      let sourceRecord: IssueSummaryRecord | undefined;
      for (const q of all) {
        const data = q.state.data as ListIssuesResponse[] | undefined;
        if (!data) continue;
        for (const page of data) {
          const found = page.issues.find((r) => r.issue_id === issueId);
          if (found) {
            sourceRecord = found;
            break;
          }
        }
        if (sourceRecord) break;
      }

      const snapshots: Array<{
        key: readonly unknown[];
        data: unknown;
      }> = [];

      if (!sourceRecord) {
        return { snapshots };
      }

      const updatedRecord: IssueSummaryRecord = {
        ...sourceRecord,
        issue: {
          ...sourceRecord.issue,
          project_id: targetProjectId,
          status: { ...sourceRecord.issue.status, key: targetStatusKey },
        },
      };

      for (const q of all) {
        const key = q.queryKey;
        const filters = (key as readonly unknown[])[1] as
          | (IssueFilters & { project_id?: string; status?: string })
          | undefined;
        if (!filters || typeof filters !== "object") continue;
        const data = q.state.data as ListIssuesResponse[] | undefined;
        if (!data) continue;
        const matchesSource =
          filters.project_id === sourceProjectId &&
          filters.status === sourceStatusKey;
        const matchesTarget =
          filters.project_id === targetProjectId &&
          filters.status === targetStatusKey;
        if (matchesSource) {
          snapshots.push({ key, data });
          const next = data.map((p) => ({
            ...p,
            issues: p.issues.filter((r) => r.issue_id !== issueId),
          }));
          queryClient.setQueryData(key, next);
        } else if (matchesTarget) {
          snapshots.push({ key, data });
          // Dedupe before prepending in case the issue is somehow already
          // in the target cell's cache (e.g. an SSE invalidate raced us).
          const dedup = data.map((p) => ({
            ...p,
            issues: p.issues.filter((r) => r.issue_id !== issueId),
          }));
          const next =
            dedup.length === 0
              ? [{ issues: [updatedRecord], next_cursor: null }]
              : [
                  {
                    ...dedup[0],
                    issues: [updatedRecord, ...dedup[0].issues],
                  },
                  ...dedup.slice(1),
                ];
          queryClient.setQueryData(key, next);
        }
      }

      return { snapshots };
    },
    onError: (err, _vars, context) => {
      if (context) {
        for (const s of context.snapshots) {
          queryClient.setQueryData(s.key, s.data);
        }
      }
      addToast(
        err instanceof Error ? err.message : "Failed to move issue",
        "error",
      );
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
    },
  });

  const handleIssueDrop = useCallback<IssueDropHandler>(
    (payload, target) => {
      if (
        payload.sourceProjectId === target.projectId &&
        payload.sourceStatusKey === target.statusKey
      ) {
        return;
      }
      moveIssue.mutate({
        issueId: payload.issueId,
        sourceProjectId: payload.sourceProjectId,
        sourceStatusKey: payload.sourceStatusKey,
        targetProjectId: target.projectId,
        targetStatusKey: target.statusKey,
      });
    },
    [moveIssue],
  );

  const projectSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const projectSortableIds = useMemo(
    () => projects.map((p) => p.project_id),
    [projects],
  );

  const handleProjectDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const ordered = projects
        .map((p) => projectRecordById.get(p.project_id))
        .filter((rec): rec is ProjectRecord => Boolean(rec));
      const oldIdx = ordered.findIndex((r) => r.project_id === active.id);
      const newIdx = ordered.findIndex((r) => r.project_id === over.id);
      if (oldIdx < 0 || newIdx < 0) return;
      const moved = ordered[oldIdx];
      const next = arrayMove(ordered, oldIdx, newIdx);
      const priority = computeReorderPriority(
        next[newIdx - 1],
        next[newIdx + 1],
      );
      projectReorder.mutate({ projectRecord: moved, priority });
    },
    [projects, projectRecordById, projectReorder],
  );

  const sections = projects.map((project) => {
    const perStatus = cells.get(project.project_id);
    const projectIssueCount = project.statuses.reduce((acc, s) => {
      const cell = perStatus?.get(s.key);
      return acc + (cell?.issues.length ?? 0);
    }, 0);
    const projectRecord = projectRecordById.get(project.project_id)!;
    const sectionProps: ProjectSectionProps = {
      project,
      projectRecord,
      perStatus,
      projectIssueCount,
      neighborhoodMap,
      hideIssues,
      collapsed: isCollapsed(project.project_id),
      onToggleCollapsed: onToggleCollapse,
      dragActive: activeProjectId !== null,
      onCardClick: handleCardClick,
      onOpenSettings: setSettingsProjectId,
      onGearClick: handleGearClick,
      onAddStatus: setNewStatusProjectId,
      onAddIssue: handleAddIssueClick,
      onIssueDrop: handleIssueDrop,
    };
    return allowProjectReorder ? (
      <SortableProjectSection key={project.project_id} {...sectionProps} />
    ) : (
      <ProjectSection key={project.project_id} {...sectionProps} />
    );
  });

  const activeProject = activeProjectId
    ? projects.find((p) => p.project_id === activeProjectId) ?? null
    : null;
  const activeIssueCount = activeProject
    ? activeProject.statuses.reduce((acc, s) => {
        const cell = cells.get(activeProject.project_id)?.get(s.key);
        return acc + (cell?.issues.length ?? 0);
      }, 0)
    : 0;

  return (
    <div className={styles.kanban}>
      {allowProjectReorder ? (
        <DndContext
          sensors={projectSensors}
          collisionDetection={closestCenter}
          // The dragged section collapses to its bar mid-drag, so the
          // surrounding sections shift. Re-measure droppables continuously,
          // otherwise drop targets resolve against the pre-collapse layout
          // and the drop snaps back to the original slot.
          measuring={{ droppable: { strategy: MeasuringStrategy.Always } }}
          onDragStart={(event) => setActiveProjectId(String(event.active.id))}
          onDragEnd={(event) => {
            setActiveProjectId(null);
            handleProjectDragEnd(event);
          }}
          onDragCancel={() => setActiveProjectId(null)}
        >
          <SortableContext
            items={projectSortableIds}
            strategy={verticalListSortingStrategy}
          >
            {sections}
          </SortableContext>
          <DragOverlay modifiers={[snapHeaderToCursorY]}>
            {activeProject ? (
              <ProjectDragPreview
                project={activeProject}
                issueCount={activeIssueCount}
                hideIssues={hideIssues}
              />
            ) : null}
          </DragOverlay>
        </DndContext>
      ) : (
        sections
      )}
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
  projectRecord: ProjectRecord;
  perStatus: Map<string, BoardCellQuery> | undefined;
  projectIssueCount: number;
  neighborhoodMap: Map<string, IssueNeighborhood>;
  hideIssues: boolean;
  collapsed: boolean;
  onToggleCollapsed: (projectId: string) => void;
  // True while any project section is being dragged. Every section collapses
  // to just its bar so the reorder list is a row of uniform-height headers —
  // far more reliable for dnd-kit than reordering full-height sections.
  dragActive: boolean;
  onCardClick: (id: string) => void;
  onOpenSettings: (id: ProjectId) => void;
  onGearClick: (
    projectRecord: ProjectRecord,
    statusKey: string,
    cell: BoardCellQuery | undefined,
  ) => void;
  onAddStatus: (projectId: string) => void;
  onAddIssue: (projectId: string, statusKey: string) => void;
  onIssueDrop: IssueDropHandler;
}

interface SortableSectionHandleProps {
  sortableSetNodeRef?: (node: HTMLElement | null) => void;
  sortableStyle?: React.CSSProperties;
  sortableIsDragging?: boolean;
  sortableDragHandleProps?: React.HTMLAttributes<HTMLElement>;
}

function SortableProjectSection(props: ProjectSectionProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } =
    useSortable({ id: props.project.project_id });
  const style: React.CSSProperties = {
    // While dragging, the moving preview is rendered by <DragOverlay>; leave
    // the source section in place (no self-transform) as a dimmed placeholder
    // so neighbors reflow around a stable slot instead of a stretching one.
    transform: isDragging ? undefined : transformToCss(transform),
    transition: transition ?? undefined,
    // Hide the source entirely while dragging; the only thing visible is the
    // header in <DragOverlay> following the cursor. The collapsed (bar-only)
    // node stays mounted so dnd-kit keeps a drop slot to reorder against.
    opacity: isDragging ? 0 : undefined,
  };
  return (
    <ProjectSection
      {...props}
      sortableSetNodeRef={setNodeRef}
      sortableStyle={style}
      sortableIsDragging={isDragging}
      sortableDragHandleProps={{ ...attributes, ...listeners }}
    />
  );
}

// Compact, fixed-size header shown inside <DragOverlay> while a project
// section is being dragged. Mirrors the project bar's left side but never
// resizes — it's decoupled from the sortable layout.
function ProjectDragPreview({
  project,
  issueCount,
  hideIssues,
}: {
  project: BoardProjectDescriptor;
  issueCount: number;
  hideIssues: boolean;
}) {
  return (
    <div className={`${styles.projectBar} ${styles.projectBarDragPreview}`}>
      <div className={styles.projectBarLeft}>
        <ProjectChip projectKey={project.key} name={project.name} />
        {!hideIssues && (
          <span className={styles.projectMeta}>
            {issueCount} {issueCount === 1 ? "issue" : "issues"}
          </span>
        )}
        <span className={styles.projectMeta}>
          {project.statuses.length}{" "}
          {project.statuses.length === 1 ? "status" : "statuses"}
        </span>
      </div>
    </div>
  );
}

function ProjectSection({
  project,
  projectRecord,
  perStatus,
  projectIssueCount,
  neighborhoodMap,
  hideIssues,
  collapsed,
  onToggleCollapsed,
  dragActive,
  onCardClick,
  onOpenSettings,
  onGearClick,
  onAddStatus,
  onAddIssue,
  onIssueDrop,
  sortableSetNodeRef,
  sortableStyle,
  sortableIsDragging,
  sortableDragHandleProps,
}: ProjectSectionProps & SortableSectionHandleProps) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const reorderMutation = useMutation({
    mutationFn: async ({
      nextStatuses,
    }: {
      nextStatuses: StatusDefinition[];
      previous: ListProjectsResponse | undefined;
    }) => {
      // Per-status PUTs persist the new `position` values. The
      // drag-to-reorder UI recomputes positions to `index * 100`
      // before mutating, so a stale cached project still sorts
      // correctly on next read. We fire requests sequentially to keep
      // mock-server ordering predictable; in production the server's
      // version-bump-per-call already serializes them.
      const ref = projectRecord.project_id;
      let lastVersion = projectRecord.version;
      for (const status of nextStatuses) {
        const resp = await apiClient.updateProjectStatus(ref, status.key, status);
        lastVersion = resp.version;
      }
      return { project_id: ref, version: lastVersion };
    },
    onError: (err, { previous }) => {
      if (previous) {
        queryClient.setQueryData(PROJECTS_QUERY_KEY, previous);
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
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const statuses = projectRecord.project.statuses;
      const oldIdx = statuses.findIndex((s) => s.key === active.id);
      const newIdx = statuses.findIndex((s) => s.key === over.id);
      if (oldIdx < 0 || newIdx < 0) return;
      // Recompute `position` to `index * 100` after the reorder so
      // the persisted column order matches the optimistic UI.
      const reorderedRaw = arrayMove(statuses, oldIdx, newIdx);
      const next: StatusDefinition[] = reorderedRaw.map((s, i) => ({
        ...s,
        position: i * 100,
      }));
      // Apply the optimistic reorder synchronously here, inside the drop event
      // handler, so React batches it into the SAME commit as dnd-kit clearing
      // the drag transform. Doing it in the mutation's onMutate instead defers
      // it past a microtask, producing a one-frame flash where the column snaps
      // back to its original slot before the reorder lands.
      const previous =
        queryClient.getQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY);
      if (previous) {
        const nextProject = { ...projectRecord.project, statuses: next };
        queryClient.setQueryData<ListProjectsResponse>(PROJECTS_QUERY_KEY, {
          projects: applyOptimisticUpsert(
            previous.projects,
            projectRecord.project_id,
            nextProject,
          ),
        });
      }
      reorderMutation.mutate({ nextStatuses: next, previous });
    },
    [projectRecord, reorderMutation, queryClient],
  );

  const columns = project.statuses.map((status) => {
    const cell = perStatus?.get(status.key);
    return (
      <SortableBoardColumn
        key={status.key}
        project={project}
        projectRecord={projectRecord}
        status={status}
        cell={cell}
        neighborhoodMap={neighborhoodMap}
        hideIssues={hideIssues}
        onCardClick={onCardClick}
        onGearClick={onGearClick}
        onAddIssue={onAddIssue}
        onIssueDrop={onIssueDrop}
      />
    );
  });

  const columnsRow = (
    <div className={styles.projectColumns}>
      {columns}
      <button
        type="button"
        className={styles.colGhost}
        onClick={() => onAddStatus(projectRecord.project_id)}
        aria-label={`Add status to ${project.name}`}
        title="Add a status to this project"
        data-testid={`board-col-add-${project.key}`}
      >
        <Icons.IconPlus size={14} />
        <span>Add status</span>
      </button>
    </div>
  );

  const sectionClasses = [styles.projectGroup];
  if (sortableIsDragging) sectionClasses.push(styles.projectGroupDragging);
  const barClasses = [styles.projectBar];
  if (sortableDragHandleProps) barClasses.push(styles.projectBarDraggable);

  return (
    <section
      ref={sortableSetNodeRef}
      style={sortableStyle}
      className={sectionClasses.join(" ")}
      data-testid={`board-project-${project.key}`}
    >
      <div
        className={barClasses.join(" ")}
        data-testid={`board-project-bar-${project.key}`}
        {...(sortableDragHandleProps ?? {})}
      >
        <button
          type="button"
          className={
            collapsed
              ? `${styles.projectCollapseToggle} ${styles.projectCollapseToggleCollapsed}`
              : styles.projectCollapseToggle
          }
          onClick={(e) => {
            // The whole bar is the drag handle; suppress propagation so the
            // dnd-kit listeners can't misread a chevron click as a tiny drag.
            e.stopPropagation();
            onToggleCollapsed(project.project_id);
          }}
          aria-expanded={!collapsed}
          aria-label={
            collapsed
              ? `Expand project ${project.name}`
              : `Collapse project ${project.name}`
          }
          title={collapsed ? "Expand project" : "Collapse project"}
          data-testid={`board-project-collapse-${project.key}`}
        >
          <Icons.IconChevronDown size={14} />
        </button>
        <div className={styles.projectBarLeft}>
          <ProjectChip
            projectKey={project.key}
            name={project.name}
            data-testid={`board-project-chip-${project.key}`}
          />
          {!hideIssues && (
            <span className={styles.projectMeta}>
              {projectIssueCount} {projectIssueCount === 1 ? "issue" : "issues"}
            </span>
          )}
          <span className={styles.projectMeta}>
            {project.statuses.length}{" "}
            {project.statuses.length === 1 ? "status" : "statuses"}
          </span>
        </div>
        <div
          className={styles.projectBarRight}
          data-testid={`board-project-actions-${project.key}`}
        >
          <button
            type="button"
            className={styles.projectSettingsButton}
            onClick={() => onOpenSettings(project.project_id)}
            title="Project settings"
            aria-label={`Project settings for ${project.name}`}
            data-testid={`board-project-settings-${project.key}`}
          >
            <Icons.IconSettings size={14} />
          </button>
        </div>
      </div>
      {/* While any project is being dragged, every section collapses to just
          the bar above. This keeps the reorder list a row of uniform-height
          headers (reliable drop targets) and hides the body of the section
          that's travelling with the cursor inside <DragOverlay>. */}
      {!dragActive && (
        <div
          className={
            collapsed
              ? `${styles.projectGroupBody} ${styles.projectGroupBodyCollapsed}`
              : styles.projectGroupBody
          }
          data-testid={`board-project-body-${project.key}`}
          aria-hidden={collapsed}
        >
          <div className={styles.projectGroupBodyInner}>
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
          </div>
        </div>
      )}
    </section>
  );
}

interface BoardColumnProps {
  project: BoardProjectDescriptor;
  projectRecord: ProjectRecord;
  status: StatusDefinition;
  cell: BoardCellQuery | undefined;
  neighborhoodMap: Map<string, IssueNeighborhood>;
  hideIssues: boolean;
  onCardClick: (id: string) => void;
  onGearClick: (
    projectRecord: ProjectRecord,
    statusKey: string,
    cell: BoardCellQuery | undefined,
  ) => void;
  onAddIssue: (projectId: string, statusKey: string) => void;
  onIssueDrop: IssueDropHandler;
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
  } = useSortable({ id: props.status.key, animateLayoutChanges: () => false });
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
      dragHandleProps={{ ...attributes, ...listeners }}
    />
  );
}

function BoardColumn({
  project,
  projectRecord,
  status,
  cell,
  neighborhoodMap,
  hideIssues,
  onCardClick,
  onGearClick,
  onAddIssue,
  onIssueDrop,
  setNodeRef,
  style,
  isDragging,
  dragHandleProps,
}: BoardColumnProps & SortableHandleProps) {
  const colIssues = cell?.issues ?? [];
  const showInitialLoading = (cell?.isLoading ?? false) && colIssues.length === 0;
  const assignTo = status.on_enter?.assign_to ?? null;
  const isInteractive = status.interactive === true;
  const colClasses = [styles.col];
  if (isDragging) colClasses.push(styles.colDragging);

  const handleColumnDragOver = (e: React.DragEvent<HTMLDivElement>) => {
    if (!e.dataTransfer.types.includes(ISSUE_CARD_DRAG_MIME)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
  };

  const handleColumnDrop = (e: React.DragEvent<HTMLDivElement>) => {
    const raw = e.dataTransfer.getData(ISSUE_CARD_DRAG_MIME);
    if (!raw) return;
    e.preventDefault();
    let payload: IssueDragPayload;
    try {
      payload = JSON.parse(raw) as IssueDragPayload;
    } catch {
      return;
    }
    onIssueDrop(payload, {
      projectId: project.project_id,
      statusKey: status.key,
    });
  };

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={colClasses.join(" ")}
      data-testid={`board-col-${project.key}-${status.key}`}
      onDragOver={handleColumnDragOver}
      onDrop={handleColumnDrop}
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
        <StatusChip status={status} />
        {!hideIssues && (
          <span className={styles.colCount}>{colIssues.length}</span>
        )}
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
          className={
            isInteractive
              ? `${styles.modeBadge} ${styles.modeBadgeInteractive}`
              : styles.modeBadge
          }
          title={
            isInteractive
              ? "Interactive — human in the loop"
              : "Autonomous agent work"
          }
          data-testid={`board-col-mode-${project.key}-${status.key}`}
        >
          {isInteractive ? (
            <>
              <Icons.IconSpark size={11} />
              interactive
            </>
          ) : (
            "auto"
          )}
        </span>
      </div>
      <div className={styles.colBody}>
        {!hideIssues && showInitialLoading && (
          <div className={styles.colEmpty}>Loading…</div>
        )}
        {colIssues.map((rec) => {
          const issue = rec.issue;
          const id = rec.issue_id;
          const pill = computeFlowPillState(neighborhoodMap.get(id));
          const archived = issue.deleted === true;
          const cardClass = archived
            ? `${styles.card} ${styles.cardArchived}`
            : styles.card;

          const handleCardDragStart = (
            e: React.DragEvent<HTMLDivElement>,
          ) => {
            const payload: IssueDragPayload = {
              issueId: id,
              sourceProjectId: project.project_id,
              sourceStatusKey: status.key,
            };
            e.dataTransfer.effectAllowed = "move";
            e.dataTransfer.setData(
              ISSUE_CARD_DRAG_MIME,
              JSON.stringify(payload),
            );
          };
          return (
            <div
              key={id}
              className={cardClass}
              onClick={() => onCardClick(id)}
              draggable
              onDragStart={handleCardDragStart}
              data-testid={`board-card-${id}`}
              data-archived={archived ? "true" : undefined}
            >
              {(archived || (issue.type && issue.type !== "unknown")) && (
                <div className={styles.cardHead}>
                  {issue.type && issue.type !== "unknown" && (
                    <TypeChip type={issue.type} />
                  )}
                  {archived && (
                    <span
                      className={styles.cardArchivedTag}
                      data-testid={`board-card-archived-${id}`}
                    >
                      ARCHIVED
                    </span>
                  )}
                  {archived && (
                    <RestoreIssueButton
                      issueId={id}
                      className={styles.cardRestoreButton}
                      data-testid={`board-card-restore-${id}`}
                    />
                  )}
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
                {pill && (
                  <FlowPill
                    phase={pill.phase}
                    num={pill.num}
                    den={pill.den}
                    title={pill.title}
                    data-testid={`board-card-flowpill-${id}`}
                  />
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
        {!hideIssues && (
          // Hover-revealed in CSS via `.col:hover`. Rendered with
          // `visibility: hidden` by default so the space is reserved and the
          // column doesn't reflow on hover. See IssuesBoard.module.css.
          <button
            type="button"
            className={styles.colAddIssue}
            onClick={() => onAddIssue(project.project_id, status.key)}
            aria-label={`Add issue to ${status.label || status.key}`}
            data-testid={`board-col-add-issue-${project.key}-${status.key}`}
          >
            <span className={styles.colAddIssueIcon}>+</span>
            Add issue
          </button>
        )}
      </div>
    </div>
  );
}
