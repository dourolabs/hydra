import { useState } from "react";
import { Link } from "react-router-dom";
import { Avatar, Badge, DiffViewer, MarkdownViewer, Panel, Tabs } from "@metis/ui";
import type { PatchVersionRecord } from "@metis/api";
import { normalizePatchStatus, normalizeCiState } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { PatchActivity } from "./PatchActivity";
import styles from "./PatchDetail.module.css";

interface PatchDetailProps {
  record: PatchVersionRecord;
  referringIssueId?: string;
}

const TABS = [
  { id: "diff", label: "Diff" },
  { id: "reviews", label: "Reviews" },
  { id: "activity", label: "Activity" },
  { id: "metadata", label: "Metadata" },
];

export function PatchDetail({ record, referringIssueId }: PatchDetailProps) {
  const [activeTab, setActiveTab] = useState("diff");
  const { patch } = record;

  return (
    <div className={styles.detail}>
      {/* Header: Title + Status */}
      <div className={styles.header}>
        <h2 className={styles.title}>{patch.title}</h2>
        <Badge status={normalizePatchStatus(patch.status)} />
      </div>

      {/* Metadata row: Branch, Base, Repository, GitHub PR, CI */}
      <div className={styles.meta}>
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
        {patch.github && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>GitHub</span>
            <span className={styles.metaValue}>
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
                <>
                  {patch.github.owner}/{patch.github.repo}#
                  {String(patch.github.number)}
                </>
              )}
            </span>
          </div>
        )}
        {patch.github?.ci && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>CI</span>
            <span className={styles.metaValue}>
              <Badge status={normalizeCiState(patch.github.ci.state)} />
              <span className={styles.ciState}>{patch.github.ci.state}</span>
              {patch.github.ci.failure && (
                <span className={styles.ciFailure}>
                  {patch.github.ci.failure.name}
                  {patch.github.ci.failure.summary &&
                    `: ${patch.github.ci.failure.summary}`}
                </span>
              )}
            </span>
          </div>
        )}
      </div>

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

      {/* Tabbed sections: Diff, Reviews, Activity, Metadata */}
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
          {activeTab === "diff" && (
            <DiffViewer diff={patch.diff} maxLines={500} />
          )}
          {activeTab === "reviews" && (
            <ReviewsList reviews={patch.reviews} />
          )}
          {activeTab === "activity" && (
            <PatchActivity patchId={record.patch_id} />
          )}
          {activeTab === "metadata" && (
            <div className={styles.metadataTab}>
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
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Patch ID</span>
                <span className={styles.metaValueMono}>{record.patch_id}</span>
              </div>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Version</span>
                <span className={styles.metaValue}>{record.version}</span>
              </div>
            </div>
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
              status={review.is_approved ? "approved" : "changes-requested"}
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
