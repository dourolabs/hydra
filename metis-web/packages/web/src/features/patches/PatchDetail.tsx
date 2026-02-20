import { useState } from "react";
import { Link } from "react-router-dom";
import { Avatar, Badge, MarkdownViewer, Panel, Tabs } from "@metis/ui";
import type { PatchVersionRecord } from "@metis/api";
import { patchToBadgeStatus, ciToBadgeStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { PatchActivity } from "./PatchActivity";
import styles from "./PatchDetail.module.css";

interface PatchDetailProps {
  record: PatchVersionRecord;
  referringIssueId?: string;
}

const TABS = [
  { id: "reviews", label: "Reviews" },
  { id: "activity", label: "Activity" },
];

export function PatchDetail({ record, referringIssueId }: PatchDetailProps) {
  const [activeTab, setActiveTab] = useState("reviews");
  const { patch } = record;

  return (
    <div className={styles.detail}>
      {/* Header: ID + Status */}
      <div className={styles.header}>
        <span className={styles.patchId}>{record.patch_id}</span>
        <Badge status={patchToBadgeStatus(patch.status)} />
      </div>

      {/* Title */}
      <h2 className={styles.title}>{patch.title}</h2>

      {/* Metadata */}
      <div className={styles.meta}>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Creator</span>
          <span className={styles.metaValue}>
            <Avatar name={patch.creator} size="sm" />
            {patch.creator}
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {formatTimestamp(record.timestamp)}
          </span>
        </div>
        {patch.branch_name && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Branch</span>
            <span className={styles.metaValueMono}>{patch.branch_name}</span>
          </div>
        )}
        {patch.base_branch && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Base</span>
            <span className={styles.metaValueMono}>{patch.base_branch}</span>
          </div>
        )}
        {patch.service_repo_name && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Repository</span>
            <span className={styles.metaValueMono}>
              {patch.service_repo_name}
            </span>
          </div>
        )}
      </div>

      {/* GitHub PR Section */}
      {patch.github && (
        <Panel
          header={<span className={styles.sectionTitle}>GitHub Pull Request</span>}
        >
          <div className={styles.sectionBody}>
            <div className={styles.ghRow}>
              <span className={styles.ghLabel}>PR</span>
              {patch.github.url ? (
                <a
                  href={patch.github.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={styles.ghLink}
                >
                  {patch.github.owner}/{patch.github.repo}#
                  {String(patch.github.number)} ↗
                </a>
              ) : (
                <span className={styles.metaValue}>
                  {patch.github.owner}/{patch.github.repo}#
                  {String(patch.github.number)}
                </span>
              )}
            </div>
            {patch.github.head_ref && (
              <div className={styles.ghRow}>
                <span className={styles.ghLabel}>Head</span>
                <span className={styles.metaValueMono}>
                  {patch.github.head_ref}
                </span>
              </div>
            )}
            {patch.github.base_ref && (
              <div className={styles.ghRow}>
                <span className={styles.ghLabel}>Base</span>
                <span className={styles.metaValueMono}>
                  {patch.github.base_ref}
                </span>
              </div>
            )}
            {patch.github.ci && (
              <div className={styles.ghRow}>
                <span className={styles.ghLabel}>CI</span>
                <Badge status={ciToBadgeStatus(patch.github.ci.state)} />
                <span className={styles.ciState}>{patch.github.ci.state}</span>
                {patch.github.ci.failure && (
                  <span className={styles.ciFailure}>
                    {patch.github.ci.failure.name}
                    {patch.github.ci.failure.summary &&
                      `: ${patch.github.ci.failure.summary}`}
                  </span>
                )}
              </div>
            )}
          </div>
        </Panel>
      )}

      {/* Description */}
      {patch.description && (
        <Panel
          header={<span className={styles.sectionTitle}>Description</span>}
        >
          <div className={styles.sectionBody}>
            <MarkdownViewer content={patch.description} />
          </div>
        </Panel>
      )}

      {/* Linked Issues */}
      {referringIssueId && (
        <Panel
          header={<span className={styles.sectionTitle}>Linked Issues</span>}
        >
          <div className={styles.sectionBody}>
            <Link
              to={`/issues/${referringIssueId}`}
              className={styles.issueLink}
            >
              {referringIssueId}
            </Link>
          </div>
        </Panel>
      )}

      {/* Tabbed sections: Reviews, Activity */}
      <Panel
        header={
          <Tabs
            tabs={TABS}
            activeTab={activeTab}
            onTabChange={setActiveTab}
          />
        }
      >
        <div className={styles.sectionBody}>
          {activeTab === "reviews" && (
            <ReviewsList reviews={patch.reviews} />
          )}
          {activeTab === "activity" && (
            <PatchActivity patchId={record.patch_id} />
          )}
        </div>
      </Panel>
    </div>
  );
}

interface ReviewsListProps {
  reviews: PatchVersionRecord["patch"]["reviews"];
}

function ReviewsList({ reviews }: ReviewsListProps) {
  if (reviews.length === 0) {
    return <p className={styles.empty}>No reviews.</p>;
  }

  return (
    <ul className={styles.reviewList}>
      {reviews.map((review, i) => (
        <li key={i} className={styles.reviewItem}>
          <div className={styles.reviewHeader}>
            <Avatar name={review.author} size="sm" />
            <span className={styles.reviewAuthor}>{review.author}</span>
            <Badge
              status={review.is_approved ? "closed" : "rejected"}
            />
            {review.submitted_at && (
              <span className={styles.reviewTime}>
                {formatTimestamp(review.submitted_at)}
              </span>
            )}
          </div>
          {review.contents && (
            <div className={styles.reviewBody}>
              <MarkdownViewer content={review.contents} />
            </div>
          )}
        </li>
      ))}
    </ul>
  );
}
