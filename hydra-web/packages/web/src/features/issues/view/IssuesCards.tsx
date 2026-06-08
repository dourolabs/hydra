import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  ProjectRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { ProjectChip } from "../../projects/ProjectChip";
import { useProjects } from "../../projects/useProjects";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import { computeBlockedStatus } from "../blockedStatus";
import styles from "./IssuesCards.module.css";

const UNRESOLVED_GROUP_KEY = "__unresolved__";

interface IssuesCardsProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  filterRootId: string | null;
}

interface ProjectSection {
  groupKey: string;
  projectKey: string;
  projectName: string | null;
  issues: IssueSummaryRecord[];
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
      issues: bucket,
    });
  }

  for (const [key, bucket] of byProject) {
    if (sections.some((s) => s.groupKey === key)) continue;
    sections.push({
      groupKey: key,
      projectKey: key === UNRESOLVED_GROUP_KEY ? "unknown" : key,
      projectName: null,
      issues: bucket,
    });
  }

  return { sections, flat: false };
}

export function IssuesCards({
  issues,
  filterRootId,
}: IssuesCardsProps) {
  const navigate = useNavigate();
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

  const handleCardClick = (id: string) => {
    navigate(`/issues/${id}${linkSearch}`);
  };

  if (flat) {
    return (
      <div className={styles.cardsWrap}>
        <div className={styles.grid}>
          {issues.map((rec) => (
            <IssueCard
              key={rec.issue_id}
              rec={rec}
              blocked={computeBlockedStatus(rec, issueMap).blocked}
              onClick={handleCardClick}
            />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className={styles.cardsWrap}>
      {sections.map((section) => (
        <section
          key={section.groupKey}
          className={styles.section}
          data-testid={`issues-cards-group-${section.projectKey}`}
        >
          <div className={styles.sectionHeader}>
            <ProjectChip
              projectKey={section.projectKey}
              name={section.projectName ?? undefined}
              data-testid={`issues-cards-group-chip-${section.projectKey}`}
            />
            <span className={styles.issueCount}>
              {section.issues.length}{" "}
              {section.issues.length === 1 ? "issue" : "issues"}
            </span>
          </div>
          <div className={styles.grid}>
            {section.issues.map((rec) => (
              <IssueCard
                key={rec.issue_id}
                rec={rec}
                blocked={computeBlockedStatus(rec, issueMap).blocked}
                onClick={handleCardClick}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

interface IssueCardProps {
  rec: IssueSummaryRecord;
  blocked: boolean;
  onClick: (id: string) => void;
}

function IssueCard({ rec, blocked, onClick }: IssueCardProps) {
  const issue = rec.issue;
  const id = rec.issue_id;
  const cardClass = blocked ? `${styles.card} ${styles.blocked}` : styles.card;
  return (
    <button
      type="button"
      className={cardClass}
      onClick={() => onClick(id)}
      data-testid={`issues-card-${id}`}
    >
      <div className={styles.cardTitle}>{issue.title || "(untitled)"}</div>
      <div className={styles.cardMeta}>
        <StatusChip
          definition={issue.resolved_status}
          fallbackKey={issue.status}
        />
        {blocked && (
          <span
            className={styles.blockedTag}
            data-testid={`issues-card-blocked-${id}`}
          >
            BLOCKED
          </span>
        )}
      </div>
      <div className={styles.cardFooter}>
        {issue.assignee ? (
          <span className={styles.assignee}>
            <Avatar
              name={principalDisplayName(issue.assignee)}
              kind={principalAvatarKind(issue.assignee)}
              size="sm"
            />
            <span className={styles.assigneeName}>
              {principalDisplayName(issue.assignee)}
            </span>
          </span>
        ) : (
          <span className={styles.dash}>—</span>
        )}
      </div>
    </button>
  );
}
