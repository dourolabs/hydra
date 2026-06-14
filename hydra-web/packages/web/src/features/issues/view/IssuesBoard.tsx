import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MeasuringStrategy,
  MouseSensor,
  TouchSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
  type Modifier,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import type {
  IssueSummaryRecord,
  ListIssuesResponse,
  ListProjectsResponse,
  Project,
  ProjectId,
  ProjectRecord,
  StatusKey,
} from "@hydra/api";
import { Picker, PickerRow } from "@hydra/ui";
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
import {
  BOARD_BULK_QUERY_KEY_MARKER,
  useBoardIssuesByProject,
  type BoardCellQuery,
  type BoardProjectDescriptor,
  type IssueFilters,
} from "../usePaginatedIssues";
import { usePageIssueTrees } from "../../dashboard/usePageIssueTrees";
import { useActiveConversationsByIssue } from "../../chat/useActiveConversationsByIssue";
import { useIsMobile } from "../../../hooks/useIsMobile";
import { useProjectCollapseState } from "./useProjectCollapseState";
import {
  ProjectDragPreview,
  ProjectSection,
  SortableProjectSection,
  type ProjectSectionProps,
} from "./ProjectSection";
import { TouchDragHoverProvider, type IssueDropHandler } from "./BoardColumn";
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

// Mobile single-board view state is mirrored to URL params (not the page-
// level FilterBar's `?project=`, which would scope the entire issues view —
// see filterUrlSync). These ride alongside the FilterBar params so a copied
// URL restores the picker selection AND the currently snapped status column
// for the recipient on mobile, and back-navigation from issue detail
// restores both without going through localStorage.
const MOBILE_PROJECT_URL_PARAM = "board_project";
const MOBILE_STATUS_URL_PARAM = "board_status";

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
  const isMobile = useIsMobile();
  const [searchParams, setSearchParams] = useSearchParams();
  const mobileSelectedProjectId = searchParams.get(MOBILE_PROJECT_URL_PARAM);
  const mobileSelectedStatusKey = searchParams.get(MOBILE_STATUS_URL_PARAM);
  const [mobilePickerOpen, setMobilePickerOpen] = useState(false);

  const setMobileSelectedProjectId = useCallback(
    (id: string | null) => {
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          if (id) {
            next.set(MOBILE_PROJECT_URL_PARAM, id);
          } else {
            next.delete(MOBILE_PROJECT_URL_PARAM);
          }
          return next;
        },
        { replace: true },
      );
    },
    [setSearchParams],
  );

  const setMobileSelectedStatusKey = useCallback(
    (key: string | null) => {
      setSearchParams(
        (prev) => {
          const current = prev.get(MOBILE_STATUS_URL_PARAM);
          if (current === key) return prev;
          const next = new URLSearchParams(prev);
          if (key) {
            next.set(MOBILE_STATUS_URL_PARAM, key);
          } else {
            next.delete(MOBILE_STATUS_URL_PARAM);
          }
          return next;
        },
        { replace: true },
      );
    },
    [setSearchParams],
  );

  const allProjectDescriptors: BoardProjectDescriptor[] = useMemo(() => {
    const out: BoardProjectDescriptor[] = [];
    const realProjects = (allProjects ?? []).filter(
      (record) => !record.project.archived,
    );

    // Archived statuses stay in the project record for collision checks /
    // historical resolution, but never render as active columns on the board.
    const activeStatuses = (record: ProjectRecord) =>
      record.project.statuses.filter((s) => !s.archived);

    if (baseFilters.project_id) {
      const match = realProjects.find(
        (p) => p.project_id === baseFilters.project_id,
      );
      if (match) {
        out.push({
          project_id: match.project_id,
          key: match.project.key,
          name: match.project.name,
          statuses: activeStatuses(match),
        });
      }
      return out;
    }

    for (const record of realProjects) {
      out.push({
        project_id: record.project_id,
        key: record.project.key,
        name: record.project.name,
        statuses: activeStatuses(record),
      });
    }
    return out;
  }, [allProjects, baseFilters.project_id]);

  // On mobile we render a single-project picker over `allProjectDescriptors`,
  // then narrow the visible / fetched set to just the chosen one. The
  // descriptor list above is what feeds the picker; `projects` below is what
  // actually drives `useBoardIssuesByProject` and the section render — so on
  // mobile we only fan out queries for the selected project, not all of them.
  const showMobilePicker =
    isMobile && !baseFilters.project_id && allProjectDescriptors.length > 1;

  // Keep the selected id in range if projects load late or the previously
  // selected one disappears (deleted, filter shifts). Falls back to the first
  // descriptor so the board always has *something* mounted.
  useEffect(() => {
    if (!showMobilePicker) return;
    const stillExists =
      mobileSelectedProjectId !== null &&
      allProjectDescriptors.some(
        (p) => p.project_id === mobileSelectedProjectId,
      );
    if (!stillExists) {
      setMobileSelectedProjectId(allProjectDescriptors[0].project_id);
    }
  }, [
    showMobilePicker,
    mobileSelectedProjectId,
    allProjectDescriptors,
    setMobileSelectedProjectId,
  ]);

  const projects: BoardProjectDescriptor[] = useMemo(() => {
    if (!showMobilePicker) return allProjectDescriptors;
    const id = mobileSelectedProjectId ?? allProjectDescriptors[0]?.project_id;
    const match = allProjectDescriptors.find((p) => p.project_id === id);
    return match ? [match] : allProjectDescriptors.slice(0, 1);
  }, [showMobilePicker, mobileSelectedProjectId, allProjectDescriptors]);

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

  const { neighborhoodMap, sessionsByIssue } = usePageIssueTrees(
    hideIssues ? [] : boardIssuesUnion,
  );

  const boardIssueIds = useMemo(
    () => (hideIssues ? [] : boardIssuesUnion.map((r) => r.issue_id)),
    [hideIssues, boardIssuesUnion],
  );
  const conversationsByIssue = useActiveConversationsByIssue(boardIssueIds);

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

  // Estimated active-issue count for the project-archive confirmation. The
  // board only knows about loaded pages; treat any project with paged-out
  // cells as non-empty (positive sentinel) so the confirmation hint never
  // under-reports.
  const settingsProjectIssueCount = useMemo(() => {
    if (!settingsProjectId) return 0;
    const perStatus = cells.get(settingsProjectId);
    if (!perStatus) return 0;
    let loaded = 0;
    let hasMore = false;
    for (const cell of perStatus.values()) {
      loaded += cell.issues.length;
      if (cell.hasNextPage) hasMore = true;
    }
    return hasMore ? Math.max(loaded, 1) : loaded;
  }, [cells, settingsProjectId]);

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

  const showNewProjectRow = !baseFilters.project_id && !showMobilePicker;
  // Only mount the project-reorder DnD when there's more than one project
  // to reorder *and* the board isn't scoped to a single project. Mobile
  // disables reorder entirely — the single-board view shows one project at
  // a time, and the picker is the canonical way to switch between them.
  const allowProjectReorder =
    !isMobile && !baseFilters.project_id && allProjectDescriptors.length > 1;

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
  // full Issue (so the truncated `IssueSummary` description doesn't clobber
  // the server-side body) and persists `status` + `project_id` via the
  // existing update-issue endpoint.
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

      // The `["paginatedIssues", …]` prefix is shared by three cache shapes:
      // - table-view `usePaginatedIssues` infinite query: `{ pages, pageParams }`,
      //   keyed `["paginatedIssues", filters, "sort", sortKey]`.
      // - per-expanded-cell board query: `ListIssuesResponse[]`,
      //   keyed `["paginatedIssues", cellFilters, "depth", depth]`.
      // - board bulk bucketed query: `ListIssuesResponse`,
      //   keyed `["paginatedIssues", baseFilters, "board-bulk", sort]`.
      // Categorize on `key[2]` so the optimistic surgery only touches the
      // shapes it knows; iterating the infinite-query payload as an array
      // throws and would abort the mutation before it fires.
      const all = queryClient
        .getQueryCache()
        .findAll({ queryKey: ["paginatedIssues"] });
      const cellQueries = all.filter((q) => {
        const key = q.queryKey as readonly unknown[];
        return key.length === 4 && key[2] === "depth";
      });
      const bulkQueries = all.filter((q) => {
        const key = q.queryKey as readonly unknown[];
        return key.length === 4 && key[2] === BOARD_BULK_QUERY_KEY_MARKER;
      });

      // Locate the source record so the optimistic insert into the target
      // cell carries the same summary fields as the rendered card. The bulk
      // query usually holds the source record (every cell's first
      // BOARD_PAGE_SIZE issues live there); fall back to the per-cell
      // queries in case the user has already expanded a cell.
      let sourceRecord: IssueSummaryRecord | undefined;
      for (const q of bulkQueries) {
        const data = q.state.data as ListIssuesResponse | undefined;
        if (!data) continue;
        const found = data.issues.find((r) => r.issue_id === issueId);
        if (found) {
          sourceRecord = found;
          break;
        }
      }
      if (!sourceRecord) {
        for (const q of cellQueries) {
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

      // Patch the bulk query: replace the source record with the updated
      // one in-place. The bulk → bucket grouping in `useBoardIssuesByProject`
      // will redistribute the issue to the target cell.
      for (const q of bulkQueries) {
        const key = q.queryKey;
        const data = q.state.data as ListIssuesResponse | undefined;
        if (!data) continue;
        const idx = data.issues.findIndex((r) => r.issue_id === issueId);
        if (idx < 0) continue;
        snapshots.push({ key, data });
        const nextIssues = data.issues.slice();
        nextIssues.splice(idx, 1);
        nextIssues.unshift(updatedRecord);
        queryClient.setQueryData(key, { ...data, issues: nextIssues });
      }

      // Patch any per-cell expanded queries that match the source/target.
      for (const q of cellQueries) {
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

  // Two pointer sensors so touch devices require a long-press while mouse
  // devices still get the existing 4px-threshold instant drag. dnd-kit picks
  // whichever sensor activates first for a given input, so trackpad / mouse
  // users are unaffected and touch users get a long-press gesture rather
  // than the mouse-style "move to start dragging" — which on touch competes
  // with native scrolling and either accidentally drags or accidentally
  // scrolls, depending on direction.
  const projectSensors = useSensors(
    useSensor(MouseSensor, { activationConstraint: { distance: 4 } }),
    useSensor(TouchSensor, {
      activationConstraint: { delay: 250, tolerance: 5 },
    }),
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
      sessionsByIssue,
      conversationsByIssue,
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
      hideBar: showMobilePicker,
      mobileSelectedStatusKey: isMobile ? mobileSelectedStatusKey : null,
      onMobileStatusChange: isMobile ? setMobileSelectedStatusKey : undefined,
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
    <TouchDragHoverProvider>
    <div className={styles.kanban}>
      {showMobilePicker &&
        (() => {
          const activeId =
            mobileSelectedProjectId ?? allProjectDescriptors[0]?.project_id;
          const active = allProjectDescriptors.find(
            (p) => p.project_id === activeId,
          );
          return (
            <div className={styles.mobilePicker}>
              <Picker
                label="Board"
                hideLabel
                open={mobilePickerOpen}
                onToggle={() => setMobilePickerOpen((v) => !v)}
                wide
                data-testid="board-mobile-picker"
                value={
                  active ? (
                    <span className={styles.mobilePickerValue}>
                      <span className={styles.mobilePickerKey}>{active.key}</span>
                      <span className={styles.mobilePickerName}>{active.name}</span>
                    </span>
                  ) : (
                    <span className={styles.mobilePickerEmpty}>Select board</span>
                  )
                }
              >
                {allProjectDescriptors.map((p) => (
                  <PickerRow
                    key={p.project_id}
                    active={p.project_id === activeId}
                    onClick={() => {
                      setMobileSelectedProjectId(p.project_id);
                      setMobilePickerOpen(false);
                    }}
                    data-testid={`board-mobile-picker-option-${p.key}`}
                  >
                    <span className={styles.mobilePickerKey}>{p.key}</span>
                    <span className={styles.mobilePickerName}>{p.name}</span>
                  </PickerRow>
                ))}
              </Picker>
            </div>
          );
        })()}
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
          issueCount={settingsProjectIssueCount}
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
    </TouchDragHoverProvider>
  );
}
