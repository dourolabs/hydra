import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar } from "@hydra/ui";
import type { IssueSummaryRecord } from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { ProjectChip } from "../../projects/ProjectChip";
import { useProjects } from "../../projects/useProjects";
import { computeBlockedStatus } from "../blockedStatus";
import { buildSections } from "./projectSections";
import styles from "./IssuesCards.module.css";

interface IssuesCardsProps {
  issues: IssueSummaryRecord[];
  filterRootId: string | null;
}

export function IssuesCards({ issues, filterRootId }: IssuesCardsProps) {
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

  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const toggle = (key: string) =>
    setCollapsed((prev) => ({ ...prev, [key]: !prev[key] }));

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
      {sections.map((section) => {
        const isCollapsed = collapsed[section.groupKey] === true;
        return (
          <section
            key={section.groupKey}
            className={styles.section}
            data-testid={`issues-cards-group-${section.projectKey}`}
          >
            <button
              type="button"
              className={styles.sectionHeader}
              onClick={() => toggle(section.groupKey)}
              aria-expanded={!isCollapsed}
              data-testid={`issues-cards-group-toggle-${section.projectKey}`}
            >
              <span className={styles.chevron} aria-hidden="true">
                {isCollapsed ? "▸" : "▾"}
              </span>
              <ProjectChip
                projectKey={section.projectKey}
                name={section.projectName ?? undefined}
                data-testid={`issues-cards-group-chip-${section.projectKey}`}
              />
              <span className={styles.issueCount}>
                {section.issues.length}{" "}
                {section.issues.length === 1 ? "issue" : "issues"}
              </span>
            </button>
            {!isCollapsed && (
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
            )}
          </section>
        );
      })}
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
