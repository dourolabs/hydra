import { useCallback, useMemo } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  DndContext,
  KeyboardSensor,
  MouseSensor,
  TouchSensor,
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
import { Button, Icons } from "@hydra/ui";
import type {
  ConversationSummary,
  ListProjectsResponse,
  ProjectId,
  ProjectRecord,
  SessionSummaryRecord,
  StatusDefinition,
} from "@hydra/api";
import { ProjectChip } from "../../projects/ProjectChip";
import {
  PROJECTS_QUERY_KEY,
  applyOptimisticUpsert,
} from "../../projects/projectCache";
import { apiClient } from "../../../api/client";
import { useToast } from "../../toast/useToast";
import { type IssueNeighborhood } from "../flowPill";
import type {
  BoardCellQuery,
  BoardProjectDescriptor,
} from "../usePaginatedIssues";
import {
  SortableBoardColumn,
  transformToCss,
  type IssueDropHandler,
} from "./BoardColumn";
import styles from "./IssuesBoard.module.css";

export interface ProjectSectionProps {
  project: BoardProjectDescriptor;
  projectRecord: ProjectRecord;
  perStatus: Map<string, BoardCellQuery> | undefined;
  projectIssueCount: number;
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  conversationsByIssue: Map<string, ConversationSummary>;
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

export function SortableProjectSection(props: ProjectSectionProps) {
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
export function ProjectDragPreview({
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

export function ProjectSection({
  project,
  projectRecord,
  perStatus,
  projectIssueCount,
  neighborhoodMap,
  sessionsByIssue,
  conversationsByIssue,
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

  // Mouse + Touch sensors so touch devices require a long-press hold before a
  // column drag begins. With a single PointerSensor `{ distance: 4 }`, any
  // touch drift competes with native scrolling. Splitting the sensors lets
  // mouse users keep the immediate 4px-threshold drag while touch users
  // explicitly opt in via a 250ms press.
  const sensors = useSensors(
    useSensor(MouseSensor, { activationConstraint: { distance: 4 } }),
    useSensor(TouchSensor, {
      activationConstraint: { delay: 250, tolerance: 5 },
    }),
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
        sessionsByIssue={sessionsByIssue}
        conversationsByIssue={conversationsByIssue}
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
        <Button
          variant="ghost"
          size="sm"
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
        </Button>
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
          <Button
            variant="secondary"
            size="sm"
            className={styles.projectSettingsButton}
            onClick={() => onOpenSettings(project.project_id)}
            title="Project settings"
            aria-label={`Project settings for ${project.name}`}
            data-testid={`board-project-settings-${project.key}`}
          >
            <Icons.IconSettings size={14} />
          </Button>
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
