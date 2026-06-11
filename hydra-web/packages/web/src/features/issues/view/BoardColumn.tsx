import { useEffect } from "react";
import { useSortable } from "@dnd-kit/sortable";
import { Link } from "react-router-dom";
import { Avatar, FlowPill, Icons, TypeChip } from "@hydra/ui";
import type {
  ConversationSummary,
  ProjectRecord,
  SessionSummaryRecord,
  StatusDefinition,
} from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { computeFlowPillState, type IssueNeighborhood } from "../flowPill";
import type {
  BoardCellQuery,
  BoardProjectDescriptor,
} from "../usePaginatedIssues";
import { AgoTime } from "../../../components/Runtime/Runtime";
import { useIsMobile } from "../../../hooks/useIsMobile";
import { RestoreIssueButton } from "../RestoreIssueButton";
import { CardRuntime } from "./CardRuntime";
import { useTouchDrag } from "./useTouchDrag";
import styles from "./IssuesBoard.module.css";

// Custom dataTransfer MIME for issue-card drags. Picked up by the column
// drop handler to disambiguate from arbitrary OS-level drags (file drops,
// text selections, etc.) so neither side preventDefaults the wrong drop.
export const ISSUE_CARD_DRAG_MIME = "application/x.hydra-issue-card";

export interface IssueDragPayload {
  issueId: string;
  sourceProjectId: string;
  sourceStatusKey: string;
}

export interface IssueDropTarget {
  projectId: string;
  statusKey: string;
}

export type IssueDropHandler = (
  payload: IssueDragPayload,
  target: IssueDropTarget,
) => void;

export interface BoardColumnProps {
  project: BoardProjectDescriptor;
  projectRecord: ProjectRecord;
  status: StatusDefinition;
  cell: BoardCellQuery | undefined;
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  conversationsByIssue: Map<string, ConversationSummary>;
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

export function transformToCss(
  transform: { x: number; y: number; scaleX: number; scaleY: number } | null,
): string | undefined {
  if (!transform) return undefined;
  return `translate3d(${transform.x}px, ${transform.y}px, 0) scaleX(${transform.scaleX}) scaleY(${transform.scaleY})`;
}

export function SortableBoardColumn(props: BoardColumnProps) {
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
  sessionsByIssue,
  conversationsByIssue,
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

  // Long-press touch drag uses this id to hit-test which column the finger is
  // over. The HTML5 drag-and-drop path is unchanged and stays the only flow
  // on desktop.
  const touchDropId = `${project.project_id}::${status.key}`;

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={colClasses.join(" ")}
      data-testid={`board-col-${project.key}-${status.key}`}
      data-touch-drop-id={touchDropId}
      data-project-id={project.project_id}
      data-status-key={status.key}
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
        {colIssues.map((rec) => (
          <BoardIssueCard
            key={rec.issue_id}
            record={rec}
            project={project}
            statusKey={status.key}
            neighborhood={neighborhoodMap.get(rec.issue_id)}
            sessions={sessionsByIssue.get(rec.issue_id)}
            conversation={conversationsByIssue.get(rec.issue_id)}
            onCardClick={onCardClick}
            onIssueDrop={onIssueDrop}
          />
        ))}
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

interface BoardIssueCardProps {
  record: BoardCellQuery["issues"][number];
  project: BoardProjectDescriptor;
  statusKey: string;
  neighborhood: IssueNeighborhood | undefined;
  sessions: SessionSummaryRecord[] | undefined;
  conversation: ConversationSummary | undefined;
  onCardClick: (id: string) => void;
  onIssueDrop: IssueDropHandler;
}

// Cards keep the HTML5 draggable path for desktop unchanged. Touch devices —
// where HTML5 drag events don't fire from finger input — get a long-press
// shim that synthesises an equivalent flow via `useTouchDrag`, hit-testing
// the column under the finger on release and calling the same `onIssueDrop`
// handler the desktop drop path uses.
function BoardIssueCard({
  record,
  project,
  statusKey,
  neighborhood,
  sessions,
  conversation,
  onCardClick,
  onIssueDrop,
}: BoardIssueCardProps) {
  const issue = record.issue;
  const id = record.issue_id;
  const pill = computeFlowPillState(neighborhood);
  const archived = issue.deleted === true;
  const isMobile = useIsMobile();

  const touchDrag = useTouchDrag<IssueDragPayload>({
    enabled: isMobile,
    payload: {
      issueId: id,
      sourceProjectId: project.project_id,
      sourceStatusKey: statusKey,
    },
    resolveDropTarget: (el) =>
      el ? el.closest("[data-touch-drop-id]") : null,
    onDrop: (payload, target) => {
      const targetProjectId = target.getAttribute("data-project-id");
      const targetStatusKey = target.getAttribute("data-status-key");
      if (!targetProjectId || !targetStatusKey) return;
      onIssueDrop(payload, {
        projectId: targetProjectId,
        statusKey: targetStatusKey,
      });
    },
  });

  // Toggle a highlight class on the column the finger is currently over so
  // the user has a visible drop preview during the touch-drag gesture.
  useEffect(() => {
    const hoverId = touchDrag.state.hoverTargetId;
    if (!hoverId) return;
    const el = document.querySelector(
      `[data-touch-drop-id="${CSS.escape(hoverId)}"]`,
    );
    if (!el) return;
    el.classList.add(styles.colTouchDropOver);
    return () => {
      el.classList.remove(styles.colTouchDropOver);
    };
  }, [touchDrag.state.hoverTargetId]);

  const handleCardDragStart = (e: React.DragEvent<HTMLDivElement>) => {
    const payload: IssueDragPayload = {
      issueId: id,
      sourceProjectId: project.project_id,
      sourceStatusKey: statusKey,
    };
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData(ISSUE_CARD_DRAG_MIME, JSON.stringify(payload));
  };

  // Tap-vs-drag: on touch devices the long-press hold elevates the gesture
  // into a drag. Suppress the synthetic click that would otherwise navigate
  // to the issue detail page after the drop lands.
  const handleClick = () => {
    if (touchDrag.state.isDragging) return;
    onCardClick(id);
  };

  const classes = [styles.card];
  if (archived) classes.push(styles.cardArchived);
  if (touchDrag.state.isDragging) classes.push(styles.cardTouchDragging);

  return (
    <div
      className={classes.join(" ")}
      onClick={handleClick}
      draggable
      onDragStart={handleCardDragStart}
      onTouchStart={touchDrag.handlers.onTouchStart}
      data-testid={`board-card-${id}`}
      data-archived={archived ? "true" : undefined}
    >
      {conversation && (
        <Link
          to={`/chat/${conversation.conversation_id}`}
          className={styles.cardChatButton}
          title={
            conversation.status === "idle"
              ? "Resume conversation"
              : "Join conversation"
          }
          aria-label={
            conversation.status === "idle"
              ? "Resume conversation"
              : "Join conversation"
          }
          onClick={(e) => e.stopPropagation()}
          data-conversation-status={conversation.status}
          data-testid={`board-card-conversation-${id}`}
        >
          <Icons.IconChat size={14} />
        </Link>
      )}
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
        <CardRuntime sessions={sessions} />
        <AgoTime iso={record.timestamp} />
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
}
