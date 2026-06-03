import { useCallback } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Avatar, Badge, Icons } from "@hydra/ui";
import type { SessionSummaryRecord } from "@hydra/api";
import { normalizeSessionStatus } from "../../../utils/statusMapping";
import { TokensCell } from "../TokensCell";
import { useMediaQuery } from "../../../hooks/useMediaQuery";
import { AgoTime, RunTime } from "../../../components/Runtime/Runtime";
import { useSingleSessionDuration } from "../../dashboard/useSessionDuration";
import { SessionRailRow } from "../../related/RailRow";
import { useSessionLinks } from "../useSessionLinks";
import { resolveSessionDisplay } from "../sessionDisplay";
import {
  FilterBar,
  type Filter,
  type FilterDefinitions,
} from "../../filters";
import styles from "./SessionsView.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

function SessionRuntimeCell({ record }: { record: SessionSummaryRecord }) {
  const { durationText, status } = useSingleSessionDuration(record);
  if (durationText === "—") return <span className={styles.dash}>—</span>;
  return <RunTime value={durationText} status={status} />;
}

interface SessionsViewProps {
  rows: SessionSummaryRecord[];
  isLoading: boolean;
  error: Error | null;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  eyebrow: string;
  // FilterBar state: the page owns it and persists it to URL.
  filters: Filter[];
  setFilters: (next: Filter[]) => void;
  definitions: FilterDefinitions<SessionSummaryRecord>;
  filteredCount: number;
  totalCount: number;
  searchValue: string;
  onSearchChange: (value: string) => void;
}

export function SessionsView({
  rows,
  isLoading,
  error,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  eyebrow,
  filters,
  setFilters,
  definitions,
  filteredCount,
  totalCount,
  searchValue,
  onSearchChange,
}: SessionsViewProps) {
  const navigate = useNavigate();
  const isMobile = useMediaQuery(MOBILE_QUERY);

  const { issueMap, conversationMap } = useSessionLinks(rows);

  const handleRowClick = useCallback(
    (id: string) => {
      navigate(`/sessions/${id}`);
    },
    [navigate],
  );

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) onLoadMore();
  }, [hasNextPage, isFetchingNextPage, onLoadMore]);

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>{eyebrow}</span>
          <h1 className={styles.pageTitle}>Sessions</h1>
        </div>
        <span className={styles.headSpacer} />
      </div>

      <div className={styles.toolbar}>
        <div className={styles.searchBox}>
          <span className={styles.searchIcon}>
            <Icons.IconSearch size={14} />
          </span>
          <input
            type="text"
            placeholder="Search sessions…"
            value={searchValue}
            onChange={(e) => onSearchChange(e.target.value)}
            aria-label="Search sessions"
            data-testid="sessions-search"
          />
        </div>
        <FilterBar
          filters={filters}
          setFilters={setFilters}
          definitions={definitions}
          count={filteredCount}
          total={totalCount}
        />
      </div>

      <div className={styles.body}>
        {isLoading && rows.length === 0 && (
          <div className={styles.empty}>Loading sessions…</div>
        )}

        {error && (
          <div className={styles.empty}>
            Failed to load sessions: {error.message}
          </div>
        )}

        {!isLoading && !error && rows.length === 0 && (
          <div className={styles.empty}>
            No sessions match the current filters.
          </div>
        )}

        {rows.length > 0 && isMobile && (
          <div className={styles.mobileList} data-testid="sessions-list">
            {rows.map((rec) => (
              <SessionRailRow
                key={rec.session_id}
                record={rec}
                display={resolveSessionDisplay(rec, issueMap, conversationMap)}
              />
            ))}
          </div>
        )}

        {rows.length > 0 && !isMobile && (
          <div className={styles.tableWrap}>
            <table className={styles.table} data-testid="sessions-list">
              <thead>
                <tr>
                  <th className={styles.colLinked}>Linked</th>
                  <th className={styles.colStatus}>Status</th>
                  <th className={styles.colAgent}>Agent</th>
                  <th className={styles.colDuration}>Duration</th>
                  <th className={styles.colTokens}>Tokens</th>
                  <th className={styles.colStarted}>Started</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((rec) => {
                  const s = rec.session;
                  const startedTs = s.start_time ?? s.creation_time ?? rec.timestamp;
                  const display = resolveSessionDisplay(
                    rec,
                    issueMap,
                    conversationMap,
                  );
                  return (
                    <tr
                      key={rec.session_id}
                      onClick={() => handleRowClick(rec.session_id)}
                      data-testid={`sessions-list-row-${rec.session_id}`}
                    >
                      <td className={styles.colLinked}>
                        <div className={styles.linkedCell}>
                          <span className={styles.linkedText}>
                            {display.title}
                          </span>
                          {display.issueId && (
                            <Link
                              to={`/issues/${display.issueId}`}
                              className={styles.linkedIssueLink}
                              onClick={(e) => e.stopPropagation()}
                              title={display.issueId}
                            >
                              {display.issueId}
                            </Link>
                          )}
                          {display.conversationId && (
                            <Link
                              to={`/chat/${display.conversationId}`}
                              className={styles.linkedIssueLink}
                              onClick={(e) => e.stopPropagation()}
                              title={display.conversationId}
                            >
                              {display.conversationId}
                            </Link>
                          )}
                        </div>
                      </td>
                      <td className={styles.colStatus}>
                        <Badge status={normalizeSessionStatus(s.status)} />
                      </td>
                      <td className={styles.colAgent}>
                        {display.agentName ? (
                          <span className={styles.agent}>
                            <Avatar
                              name={display.agentName}
                              kind="agent"
                              size="md"
                            />
                            <span className={styles.agentName}>
                              {display.agentName}
                            </span>
                          </span>
                        ) : (
                          <span className={styles.dash}>—</span>
                        )}
                      </td>
                      <td className={styles.colDuration}>
                        <SessionRuntimeCell record={rec} />
                      </td>
                      <td className={styles.colTokens}>
                        <TokensCell usage={s.usage} />
                      </td>
                      <td className={styles.colStarted}>
                        <AgoTime iso={startedTs} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}

        {hasNextPage && (
          <div className={styles.loadMore}>
            <button
              type="button"
              className={styles.loadMoreButton}
              onClick={handleLoadMore}
              disabled={isFetchingNextPage}
              data-testid="sessions-load-more"
            >
              {isFetchingNextPage ? "Loading…" : "Load more"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
