import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, FlowPill, TypeChip } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { ProjectChip } from "../../projects/ProjectChip";
import { useProjects } from "../../projects/useProjects";
import { useMediaQuery } from "../../../hooks/useMediaQuery";
import { AgoTime, RunTime } from "../../../components/Runtime/Runtime";
import { useSessionDuration } from "../../dashboard/useSessionDuration";
import { IssueRailRow } from "../../related/RailRow";
import { computeBlockedStatus } from "../blockedStatus";
import { computeFlowPillState, type IssueNeighborhood } from "../flowPill";
import { RestoreIssueButton } from "../RestoreIssueButton";
import { buildSections, type ProjectSection } from "./projectSections";
import styles from "./IssuesTable.module.css";

const MOBILE_QUERY = "(max-width: 768px)";
const TABLE_COLUMN_COUNT = 7;

interface IssuesTableProps {
  issues: IssueSummaryRecord[];
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  filterRootId: string | null;
}

function RuntimeCell({ sessions }: { sessions: SessionSummaryRecord[] | undefined }) {
  const { durationText, status } = useSessionDuration(sessions);
  if (durationText === "—") return <span className={styles.dash}>—</span>;
  return <RunTime value={durationText} status={status} />;
}

export function IssuesTable({
  issues,
  neighborhoodMap,
  sessionsByIssue,
  filterRootId,
}: IssuesTableProps) {
  const navigate = useNavigate();
  const isMobile = useMediaQuery(MOBILE_QUERY);
  const { data: projects } = useProjects();

  const linkSearch =
    "?" +
    new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    }).toString();

  const issueMap = useMemo(() => {
    const m = new Map<string, IssueSummaryRecord>();
    for (const rec of issues) m.set(rec.issue_id, rec);
    return m;
  }, [issues]);

  const { sections, flat } = useMemo(
    () => buildSections(issues, projects),
    [issues, projects],
  );

  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const toggle = (key: string) =>
    setCollapsed((prev) => ({ ...prev, [key]: !prev[key] }));

  const handleRowClick = (id: string) => {
    navigate(`/issues/${id}${linkSearch}`);
  };

  if (isMobile) {
    if (flat) {
      return (
        <div className={styles.mobileList}>
          {issues.map((rec) => (
            <IssueRailRow
              key={rec.issue_id}
              record={rec}
              sessions={sessionsByIssue.get(rec.issue_id)}
              neighborhood={neighborhoodMap.get(rec.issue_id)}
              linkSearch={linkSearch}
            />
          ))}
        </div>
      );
    }
    return (
      <div className={styles.mobileList}>
        {sections.map((section) => {
          const isCollapsed = collapsed[section.groupKey] === true;
          return (
            <div
              key={section.groupKey}
              className={styles.mobileGroup}
              data-testid={`issues-table-group-${section.projectKey}`}
            >
              <SectionHeader
                section={section}
                collapsed={isCollapsed}
                onToggle={() => toggle(section.groupKey)}
              />
              {!isCollapsed &&
                section.issues.map((rec) => (
                  <IssueRailRow
                    key={rec.issue_id}
                    record={rec}
                    sessions={sessionsByIssue.get(rec.issue_id)}
                    neighborhood={neighborhoodMap.get(rec.issue_id)}
                    linkSearch={linkSearch}
                  />
                ))}
            </div>
          );
        })}
      </div>
    );
  }

  if (flat) {
    return (
      <div className={styles.tableWrap}>
        <table className={styles.table}>
          <TableHead />
          <tbody>
            {issues.map((rec) => (
              <IssueDataRow
                key={rec.issue_id}
                rec={rec}
                blocked={computeBlockedStatus(rec, issueMap).blocked}
                neighborhoodMap={neighborhoodMap}
                sessionsByIssue={sessionsByIssue}
                onClick={handleRowClick}
              />
            ))}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <div className={styles.tableWrap}>
      <table className={styles.table}>
        <TableHead />
        <tbody>
          {sections.map((section) => {
            const isCollapsed = collapsed[section.groupKey] === true;
            return (
              <SectionRows
                key={section.groupKey}
                section={section}
                collapsed={isCollapsed}
                onToggle={() => toggle(section.groupKey)}
                issueMap={issueMap}
                neighborhoodMap={neighborhoodMap}
                sessionsByIssue={sessionsByIssue}
                onRowClick={handleRowClick}
              />
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function TableHead() {
  return (
    <thead>
      <tr>
        <th className={styles.colTitle}>Title</th>
        <th className={styles.colStatus}>Status</th>
        <th className={styles.colType}>Type</th>
        <th className={styles.colAssignee}>Assignee</th>
        <th className={styles.colProgress}>Progress</th>
        <th className={styles.colRuntime}>Runtime</th>
        <th className={styles.colUpdated}>Updated</th>
      </tr>
    </thead>
  );
}

interface SectionRowsProps {
  section: ProjectSection;
  collapsed: boolean;
  onToggle: () => void;
  issueMap: Map<string, IssueSummaryRecord>;
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  onRowClick: (id: string) => void;
}

function SectionRows({
  section,
  collapsed,
  onToggle,
  issueMap,
  neighborhoodMap,
  sessionsByIssue,
  onRowClick,
}: SectionRowsProps) {
  return (
    <>
      <tr
        className={styles.sectionRow}
        data-testid={`issues-table-group-${section.projectKey}`}
      >
        <td colSpan={TABLE_COLUMN_COUNT} className={styles.sectionCell}>
          <SectionHeader
            section={section}
            collapsed={collapsed}
            onToggle={onToggle}
          />
        </td>
      </tr>
      {!collapsed &&
        section.issues.map((rec) => (
          <IssueDataRow
            key={rec.issue_id}
            rec={rec}
            blocked={computeBlockedStatus(rec, issueMap).blocked}
            neighborhoodMap={neighborhoodMap}
            sessionsByIssue={sessionsByIssue}
            onClick={onRowClick}
          />
        ))}
    </>
  );
}

interface SectionHeaderProps {
  section: ProjectSection;
  collapsed: boolean;
  onToggle: () => void;
}

function SectionHeader({ section, collapsed, onToggle }: SectionHeaderProps) {
  const issueCount = section.issues.length;
  return (
    <button
      type="button"
      className={styles.sectionHeader}
      onClick={onToggle}
      aria-expanded={!collapsed}
      data-testid={`issues-table-group-toggle-${section.projectKey}`}
    >
      <span className={styles.chevron} aria-hidden="true">
        {collapsed ? "▸" : "▾"}
      </span>
      <ProjectChip
        projectKey={section.projectKey}
        name={section.projectName ?? undefined}
        data-testid={`issues-table-group-chip-${section.projectKey}`}
      />
      <span className={styles.issueCount}>
        {issueCount} {issueCount === 1 ? "issue" : "issues"}
      </span>
      <StatusPipRow section={section} />
    </button>
  );
}

function StatusPipRow({ section }: { section: ProjectSection }) {
  if (section.statuses.length === 0) return null;
  const counts = new Map<string, number>();
  for (const rec of section.issues) {
    const key = rec.issue.status.key;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return (
    <span className={styles.pipRow}>
      {section.statuses.map((status) => {
        const count = counts.get(status.key) ?? 0;
        if (count === 0) return null;
        return (
          <span
            key={status.key}
            className={styles.pip}
            data-testid={`issues-table-pip-${section.projectKey}-${status.key}`}
            title={`${status.label}: ${count}`}
          >
            <span
              className={styles.pipDot}
              style={{ backgroundColor: status.color }}
              aria-hidden="true"
            />
            <span className={styles.pipCount}>{count}</span>
          </span>
        );
      })}
    </span>
  );
}

interface IssueDataRowProps {
  rec: IssueSummaryRecord;
  blocked: boolean;
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  onClick: (id: string) => void;
}

function IssueDataRow({
  rec,
  blocked,
  neighborhoodMap,
  sessionsByIssue,
  onClick,
}: IssueDataRowProps) {
  const issue = rec.issue;
  const id = rec.issue_id;
  const pill = computeFlowPillState(neighborhoodMap.get(id));
  const archived = issue.deleted === true;
  const rowClasses = [styles.dataRow];
  if (blocked) rowClasses.push(styles.blocked);
  if (archived) rowClasses.push(styles.archived);

  return (
    <tr
      className={rowClasses.join(" ")}
      data-testid={`issues-list-row-${id}`}
      data-archived={archived ? "true" : undefined}
      onClick={() => onClick(id)}
    >
      <td className={styles.colTitle}>
        <div className={styles.titleCell}>
          <span className={styles.titleText}>{issue.title || "(untitled)"}</span>
          {archived && (
            <>
              <span
                className={styles.archivedTag}
                data-testid={`issues-row-archived-${id}`}
              >
                ARCHIVED
              </span>
              <RestoreIssueButton
                issueId={id}
                className={styles.restoreButton}
                data-testid={`issues-row-restore-${id}`}
              />
            </>
          )}
        </div>
      </td>
      <td className={styles.colStatus}>
        <div className={styles.statusCell}>
          <StatusChip status={issue.status} />
          {blocked && (
            <span
              className={styles.blockedTag}
              data-testid={`issues-row-blocked-${id}`}
            >
              BLOCKED
            </span>
          )}
        </div>
      </td>
      <td className={styles.colType}>
        {issue.type && issue.type !== "unknown" ? (
          <TypeChip type={issue.type} />
        ) : (
          <span className={styles.dash}>—</span>
        )}
      </td>
      <td className={styles.colAssignee}>
        {issue.assignee ? (
          <span className={styles.assignee}>
            <Avatar
              name={principalDisplayName(issue.assignee)}
              kind={principalAvatarKind(issue.assignee)}
              size="md"
            />
            <span className={styles.assigneeName}>
              {principalDisplayName(issue.assignee)}
            </span>
          </span>
        ) : (
          <span className={styles.dash}>—</span>
        )}
      </td>
      <td className={styles.colProgress}>
        {pill ? (
          <FlowPill
            phase={pill.phase}
            num={pill.num}
            den={pill.den}
            title={pill.title}
            data-testid={`issues-row-flowpill-${id}`}
          />
        ) : (
          <span className={styles.dash}>—</span>
        )}
      </td>
      <td className={styles.colRuntime}>
        <RuntimeCell sessions={sessionsByIssue.get(id)} />
      </td>
      <td className={styles.colUpdated}>
        <AgoTime iso={rec.timestamp} />
      </td>
    </tr>
  );
}
