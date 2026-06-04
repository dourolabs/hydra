import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, TypeChip } from "@hydra/ui";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { useProjectStatuses } from "../../projects/useProjects";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import {
  BOARD_STATUSES,
  usePaginatedIssuesByStatus,
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

export function IssuesBoard({ baseFilters, username, filterRootId }: IssuesBoardProps) {
  const navigate = useNavigate();
  const columns = usePaginatedIssuesByStatus(baseFilters);
  const { data: defaultStatuses } = useProjectStatuses(null);
  const statusByKey = useMemo(() => {
    const map = new Map<string, NonNullable<typeof defaultStatuses>["statuses"][number]>();
    for (const s of defaultStatuses?.statuses ?? []) map.set(s.key, s);
    return map;
  }, [defaultStatuses]);

  const boardIssuesUnion = useMemo(() => {
    const seen = new Set<string>();
    const out = [];
    for (const status of BOARD_STATUSES) {
      for (const rec of columns[status].issues) {
        if (seen.has(rec.issue_id)) continue;
        seen.add(rec.issue_id);
        out.push(rec);
      }
    }
    return out;
  }, [columns]);

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
      {BOARD_STATUSES.map((status) => {
        const col = columns[status];
        const colIssues = col.issues;
        const showInitialLoading = col.isLoading && colIssues.length === 0;
        return (
          <div key={status} className={styles.col}>
            <div className={styles.colHead}>
              <StatusChip definition={statusByKey.get(status)} fallbackKey={status} />
              <span className={styles.colCount}>{colIssues.length}</span>
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
                    onClick={() => handleCardClick(id)}
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
              {col.hasNextPage && (
                <div className={styles.colLoadMore}>
                  <button
                    type="button"
                    className={styles.colLoadMoreButton}
                    onClick={col.fetchNextPage}
                    disabled={col.isFetchingNextPage}
                    data-testid={`issues-board-load-more-${status}`}
                  >
                    {col.isFetchingNextPage ? "Loading…" : "Load more"}
                  </button>
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
