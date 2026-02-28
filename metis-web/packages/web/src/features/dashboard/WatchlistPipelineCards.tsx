import { useState, useMemo, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { Badge, Avatar, JobStatusIndicator } from "@metis/ui";
import type { IssueSummaryRecord, JobSummaryRecord, PatchSummaryRecord } from "@metis/api";
import { buildIssueTree, type IssueTreeNode } from "../issues/useIssues";
import { treeHasActiveNode } from "./watchingUtils";
import { toJobSummary } from "../../utils/jobMapping";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import {
  classifySubtasks,
  countActiveJobs,
  computeProgress,
  STAGE_ORDER,
  STAGE_LABELS,
  type PipelineStage,
  type PipelineStageCounts,
  type ClassifiedIssue,
} from "./pipelineUtils";
import styles from "./WatchlistPipelineCards.module.css";

interface WatchlistPipelineCardsProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  patches: PatchSummaryRecord[] | undefined;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
}

const SEGMENT_CLASSES: Record<PipelineStage, string> = {
  open: styles.segmentOpen,
  "agent-working": styles.segmentAgentWorking,
  "awaiting-review": styles.segmentAwaitingReview,
  done: styles.segmentDone,
  failed: styles.segmentFailed,
};

/* ── Tooltip on pipeline segment hover ─────────────── */

function SegmentTooltip({ stage, items }: { stage: PipelineStage; items: ClassifiedIssue[] }) {
  const maxShown = 8;
  return (
    <div className={styles.tooltip}>
      <div className={styles.tooltipTitle}>{STAGE_LABELS[stage]} ({items.length})</div>
      {items.slice(0, maxShown).map((item) => (
        <div key={item.issue.issue_id} className={styles.tooltipItem}>
          {descriptionSnippet(item.issue.issue.description, 50)}
        </div>
      ))}
      {items.length > maxShown && (
        <div className={styles.tooltipItem}>+{items.length - maxShown} more</div>
      )}
    </div>
  );
}

/* ── Pipeline bar ──────────────────────────────────── */

function PipelineBar({
  counts,
}: {
  counts: PipelineStageCounts;
}) {
  const [hoveredStage, setHoveredStage] = useState<PipelineStage | null>(null);

  const total = STAGE_ORDER.reduce((sum, s) => sum + counts[s].length, 0);
  if (total === 0) return null;

  return (
    <div className={styles.pipelineBar}>
      {STAGE_ORDER.map((stage) => {
        const items = counts[stage];
        if (items.length === 0) return null;
        const pct = (items.length / total) * 100;
        return (
          <div
            key={stage}
            className={`${styles.segment} ${SEGMENT_CLASSES[stage]}`}
            style={{ width: `${pct}%` }}
            onMouseEnter={() => setHoveredStage(stage)}
            onMouseLeave={() => setHoveredStage(null)}
          >
            {items.length}
            {hoveredStage === stage && (
              <SegmentTooltip stage={stage} items={items} />
            )}
          </div>
        );
      })}
    </div>
  );
}

/* ── Attention banner ──────────────────────────────── */

function AttentionBanner({ counts }: { counts: PipelineStageCounts }) {
  const reviewCount = counts["awaiting-review"].length;
  const failedCount = counts.failed.length;

  if (reviewCount === 0 && failedCount === 0) return null;

  const parts: string[] = [];
  if (reviewCount > 0) parts.push(`${reviewCount} awaiting your review`);
  if (failedCount > 0) parts.push(`${failedCount} failed`);

  return (
    <div className={styles.attentionBanner}>
      {parts.join(" \u00B7 ")}
    </div>
  );
}

/* ── Subtask row ───────────────────────────────────── */

function SubtaskRow({
  item,
  isSelected,
  onSelect,
  onJobClick,
}: {
  item: ClassifiedIssue;
  isSelected: boolean;
  onSelect: (id: string) => void;
  onJobClick: (issueId: string, jobId: string) => void;
}) {
  const jobSummaries = item.jobs.map(toJobSummary);
  const handleJobClick = useCallback(
    (jobId: string) => onJobClick(item.issue.issue_id, jobId),
    [onJobClick, item.issue.issue_id],
  );

  const cn = [styles.subtaskRow];
  if (isSelected) cn.push(styles.subtaskActive);

  return (
    <button
      className={cn.join(" ")}
      type="button"
      onClick={() => onSelect(item.issue.issue_id)}
    >
      <Badge status={issueToBadgeStatus(item.issue.issue.status)} />
      <span className={styles.description}>
        {descriptionSnippet(item.issue.issue.description)}
      </span>
      {jobSummaries.length > 0 && (
        <span onClick={(e) => e.stopPropagation()} role="presentation">
          <JobStatusIndicator jobs={jobSummaries} onJobClick={handleJobClick} />
        </span>
      )}
      {item.issue.issue.assignee && (
        <Avatar name={item.issue.issue.assignee} size="sm" />
      )}
    </button>
  );
}

/* ── Expandable subtask list ───────────────────────── */

function SubtaskList({
  counts,
  selectedId,
  onSelect,
  onJobClick,
}: {
  counts: PipelineStageCounts;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onJobClick: (issueId: string, jobId: string) => void;
}) {
  return (
    <div className={styles.subtaskList}>
      {STAGE_ORDER.map((stage) => {
        const items = counts[stage];
        if (items.length === 0) return null;
        return (
          <div key={stage} className={styles.stageGroup}>
            <div className={styles.stageLabel}>
              {STAGE_LABELS[stage]} ({items.length})
            </div>
            {items.map((item) => (
              <SubtaskRow
                key={item.issue.issue_id}
                item={item}
                isSelected={item.issue.issue_id === selectedId}
                onSelect={onSelect}
                onJobClick={onJobClick}
              />
            ))}
          </div>
        );
      })}
    </div>
  );
}

/* ── Pipeline card ─────────────────────────────────── */

function PipelineCard({
  root,
  jobsByIssue,
  patchMap,
  selectedId,
  onSelect,
  expanded,
  onToggle,
  onJobClick,
}: {
  root: IssueTreeNode;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  patchMap: Map<string, PatchSummaryRecord>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  expanded: boolean;
  onToggle: (id: string) => void;
  onJobClick: (issueId: string, jobId: string) => void;
}) {
  const counts = useMemo(
    () => classifySubtasks(root, jobsByIssue, patchMap),
    [root, jobsByIssue, patchMap],
  );

  const activeJobs = useMemo(
    () => countActiveJobs(root, jobsByIssue),
    [root, jobsByIssue],
  );

  const progress = computeProgress(counts);
  const totalSubtasks = STAGE_ORDER.reduce((s, st) => s + counts[st].length, 0);

  const headerCn = [styles.header];
  if (root.id === selectedId) headerCn.push(styles.active);

  return (
    <li className={styles.card}>
      <button
        className={headerCn.join(" ")}
        type="button"
        onClick={() => onSelect(root.id)}
      >
        <span
          className={styles.chevron}
          onClick={(e) => {
            e.stopPropagation();
            onToggle(root.id);
          }}
          role="button"
          tabIndex={-1}
        >
          {totalSubtasks > 0 ? (expanded ? "\u25BE" : "\u25B8") : " "}
        </span>
        <span className={styles.description}>
          {descriptionSnippet(root.issue.issue.description)}
        </span>
        {totalSubtasks > 0 && (
          <span className={styles.progress}>{progress}%</span>
        )}
        <span className={styles.activityIndicator}>
          <span className={activeJobs > 0 ? styles.activityDot : styles.activityDotIdle} />
          {activeJobs > 0 ? `${activeJobs} active` : "idle"}
        </span>
      </button>
      {totalSubtasks > 0 && <PipelineBar counts={counts} />}
      <AttentionBanner counts={counts} />
      {expanded && (
        <SubtaskList
          counts={counts}
          selectedId={selectedId}
          onSelect={onSelect}
          onJobClick={onJobClick}
        />
      )}
    </li>
  );
}

/* ── Main component ────────────────────────────────── */

export function WatchlistPipelineCards({
  issues,
  jobsByIssue,
  patches,
  selectedId,
  onSelect,
  username,
}: WatchlistPipelineCardsProps) {
  const navigate = useNavigate();
  const [expandedCards, setExpandedCards] = useState<Set<string>>(new Set());

  const handleJobClick = useCallback(
    (issueId: string, jobId: string) => {
      navigate(`/issues/${issueId}/jobs/${jobId}/logs`);
    },
    [navigate],
  );

  const toggleCard = useCallback((id: string) => {
    setExpandedCards((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const patchMap = useMemo(() => {
    const map = new Map<string, PatchSummaryRecord>();
    if (patches) {
      for (const p of patches) {
        map.set(p.patch_id, p);
      }
    }
    return map;
  }, [patches]);

  const watchingRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked &&
          root.issue.issue.creator === username &&
          treeHasActiveNode(root, jobsByIssue),
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, jobsByIssue, username]);

  if (watchingRoots.length === 0) {
    return <p className={styles.empty}>No issues being watched.</p>;
  }

  return (
    <ul className={styles.list}>
      {watchingRoots.map((root) => (
        <PipelineCard
          key={root.id}
          root={root}
          jobsByIssue={jobsByIssue}
          patchMap={patchMap}
          selectedId={selectedId}
          onSelect={onSelect}
          expanded={expandedCards.has(root.id)}
          onToggle={toggleCard}
          onJobClick={handleJobClick}
        />
      ))}
    </ul>
  );
}
