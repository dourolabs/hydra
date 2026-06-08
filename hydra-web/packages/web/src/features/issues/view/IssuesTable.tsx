import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, TypeChip } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  ProjectRecord,
  SessionSummaryRecord,
  StatusDefinition,
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
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import { useSessionDuration } from "../../dashboard/useSessionDuration";
import { IssueRailRow } from "../../related/RailRow";
import { computeBlockedStatus } from "../blockedStatus";
import styles from "./IssuesTable.module.css";

const MOBILE_QUERY = "(max-width: 768px)";
// Sentinel group key for issues whose `project_id` doesn't resolve to a
// known project (e.g. references a project not returned by useProjects()).
// Real project ids never collide with this string.
const UNRESOLVED_GROUP_KEY = "__unresolved__";
const TABLE_COLUMN_COUNT = 7;

interface IssuesTableProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  filterRootId: string | null;
}

interface ProjectSection {
  groupKey: string;
  projectKey: string;
  projectName: string | null;
  statuses: StatusDefinition[];
  issues: IssueSummaryRecord[];
}

function progressFraction(children: ChildStatus[] | undefined): number {
  if (!children || children.length === 0) return 0;
  const total = children.length;
  const projected = children.filter(
    (c) => c.status === "closed" || c.status === "issue-closed" || c.status === "in-progress",
  ).length;
  return Math.round((projected / total) * 100);
}

function RuntimeCell({ sessions }: { sessions: SessionSummaryRecord[] | undefined }) {
  const { durationText, status } = useSessionDuration(sessions);
  if (durationText === "—") return <span className={styles.dash}>—</span>;
  return <RunTime value={durationText} status={status} />;
}

function findDefaultProject(
  projects: ProjectRecord[] | undefined,
): ProjectRecord | null {
  if (!projects || projects.length === 0) return null;
  return projects.find((p) => p.project.key === "default") ?? null;
}

function buildSections(
  issues: IssueSummaryRecord[],
  projects: ProjectRecord[] | undefined,
): { sections: ProjectSection[]; flat: boolean } {
  if (!projects || projects.length === 0) {
    return { sections: [], flat: true };
  }
  const defaultProject = findDefaultProject(projects);

  const byProject = new Map<string, IssueSummaryRecord[]>();
  for (const rec of issues) {
    const projectId = rec.issue.project_id ?? null;
    const key =
      projectId ?? (defaultProject?.project_id ?? UNRESOLVED_GROUP_KEY);
    const bucket = byProject.get(key);
    if (bucket) {
      bucket.push(rec);
    } else {
      byProject.set(key, [rec]);
    }
  }

  const ordered: ProjectRecord[] = [];
  if (defaultProject) ordered.push(defaultProject);
  for (const p of projects) {
    if (p === defaultProject) continue;
    ordered.push(p);
  }

  const sections: ProjectSection[] = [];
  for (const project of ordered) {
    const bucket = byProject.get(project.project_id);
    if (!bucket || bucket.length === 0) continue;
    sections.push({
      groupKey: project.project_id,
      projectKey: project.project.key,
      projectName: project.project.name,
      statuses: project.project.statuses,
      issues: bucket,
    });
  }

  for (const [key, bucket] of byProject) {
    if (sections.some((s) => s.groupKey === key)) continue;
    sections.push({
      groupKey: key,
      projectKey: key === UNRESOLVED_GROUP_KEY ? "unknown" : key,
      projectName: null,
      statuses: [],
      issues: bucket,
    });
  }

  return { sections, flat: false };
}

export function IssuesTable({
  issues,
  childStatusMap,
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
              childStatuses={childStatusMap.get(rec.issue_id)}
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
                    childStatuses={childStatusMap.get(rec.issue_id)}
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
                childStatusMap={childStatusMap}
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
                childStatusMap={childStatusMap}
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
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  onRowClick: (id: string) => void;
}

function SectionRows({
  section,
  collapsed,
  onToggle,
  issueMap,
  childStatusMap,
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
            childStatusMap={childStatusMap}
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
    const key = rec.issue.status;
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
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  onClick: (id: string) => void;
}

function IssueDataRow({
  rec,
  blocked,
  childStatusMap,
  sessionsByIssue,
  onClick,
}: IssueDataRowProps) {
  const issue = rec.issue;
  const id = rec.issue_id;
  const children = childStatusMap.get(id);
  const pct = progressFraction(children);
  const hasActiveChild = !!children?.some((c) => c.hasActiveTask);
  const progressClass = hasActiveChild
    ? `${styles.progress} ${styles.progressActive}`
    : styles.progress;
  const fillClass = hasActiveChild
    ? `${styles.progressFill} ${styles.progressFillActive}`
    : styles.progressFill;
  const rowClass = blocked ? `${styles.dataRow} ${styles.blocked}` : styles.dataRow;

  return (
    <tr
      className={rowClass}
      data-testid={`issues-list-row-${id}`}
      onClick={() => onClick(id)}
    >
      <td className={styles.colTitle}>
        <div className={styles.titleCell}>
          <span className={styles.titleText}>{issue.title || "(untitled)"}</span>
        </div>
      </td>
      <td className={styles.colStatus}>
        <div className={styles.statusCell}>
          <StatusChip
            definition={issue.resolved_status}
            fallbackKey={issue.status}
          />
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
        {children && children.length > 0 ? (
          <div className={progressClass} title={`${pct}%`}>
            <span className={fillClass} style={{ width: `${pct}%` }} />
          </div>
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
