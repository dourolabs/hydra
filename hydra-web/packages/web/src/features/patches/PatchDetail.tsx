import { useState } from "react";
import { Link } from "react-router-dom";
import { Avatar, Badge, DiffViewer } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import type { PatchVersionRecord } from "@hydra/api";
import { normalizePatchStatus, normalizeCiState } from "../../utils/badgeStatus";
import { formatTimestamp } from "../../utils/time";
import { PatchActivity } from "./PatchActivity";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../principal/formatPrincipal";
import styles from "./PatchDetail.module.css";

interface PatchDetailProps {
  record: PatchVersionRecord;
  referringIssueId?: string;
}

type TabKey = "diff" | "reviews" | "activity";

const TABS: { key: TabKey; label: string }[] = [
  { key: "diff", label: "Diff" },
  { key: "reviews", label: "Reviews" },
  { key: "activity", label: "Activity" },
];

export function PatchDetail({ record, referringIssueId }: PatchDetailProps) {
  const [activeTab, setActiveTab] = useState<TabKey>("diff");
  const { patch } = record;

  const status =
    patch.status === "open" && patch.reviews.some((r) => r.is_approved)
      ? "approved"
      : normalizePatchStatus(patch.status);

  return (
    <div className={styles.page}>
      <div className={styles.inner}>
        {/* Title block */}
        <div className={styles.titleRow}>
          <span className={styles.titleId}>{record.patch_id}</span>
          <Badge status={status} />
        </div>
        <h1 className={styles.title}>{patch.title || record.patch_id}</h1>

        {/* Key-vals grid */}
        <div className={styles.keyvals}>
          <div className={styles.keyval}>
            <span className={styles.keyvalLabel}>Author</span>
            <span className={styles.keyvalValue}>
              <Avatar name={patch.creator} size="md" />
              <span className={styles.keyvalText}>{patch.creator}</span>
            </span>
          </div>
          <div className={styles.keyval}>
            <span className={styles.keyvalLabel}>Repository</span>
            <span className={`${styles.keyvalValue} ${styles.keyvalValueMono}`}>
              <span className={styles.keyvalText}>{patch.service_repo_name}</span>
            </span>
          </div>
          {patch.branch_name && (
            <div className={styles.keyval}>
              <span className={styles.keyvalLabel}>Branch</span>
              <span className={`${styles.keyvalValue} ${styles.keyvalValueMono}`}>
                <span className={styles.keyvalText}>{patch.branch_name}</span>
              </span>
            </div>
          )}
          {patch.base_branch && (
            <div className={styles.keyval}>
              <span className={styles.keyvalLabel}>Base</span>
              <span className={`${styles.keyvalValue} ${styles.keyvalValueMono}`}>
                <span className={styles.keyvalText}>{patch.base_branch}</span>
              </span>
            </div>
          )}
          <div className={styles.keyval}>
            <span className={styles.keyvalLabel}>Created</span>
            <span className={`${styles.keyvalValue} ${styles.keyvalValueMono}`}>
              {formatTimestamp(record.creation_time ?? record.timestamp)}
            </span>
          </div>
          <div className={styles.keyval}>
            <span className={styles.keyvalLabel}>Updated</span>
            <span className={`${styles.keyvalValue} ${styles.keyvalValueMono}`}>
              {formatTimestamp(record.timestamp)}
            </span>
          </div>
          {referringIssueId && (
            <div className={styles.keyval}>
              <span className={styles.keyvalLabel}>Linked issue</span>
              <Link to={`/issues/${referringIssueId}`} className={`${styles.keyvalValue} ${styles.linkAcc} ${styles.keyvalValueMono}`}>
                <span className={styles.keyvalText}>{referringIssueId}</span>
              </Link>
            </div>
          )}
          {patch.github && (
            <div className={styles.keyval}>
              <span className={styles.keyvalLabel}>GitHub</span>
              {patch.github.url ? (
                <a
                  href={patch.github.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={`${styles.keyvalValue} ${styles.linkAcc} ${styles.keyvalValueMono}`}
                >
                  <span className={styles.keyvalText}>
                    {patch.github.owner}/{patch.github.repo}#{String(patch.github.number)}
                  </span>
                </a>
              ) : (
                <span className={`${styles.keyvalValue} ${styles.keyvalValueMono}`}>
                  <span className={styles.keyvalText}>
                    {patch.github.owner}/{patch.github.repo}#{String(patch.github.number)}
                  </span>
                </span>
              )}
            </div>
          )}
          {patch.github?.ci && (
            <div className={styles.keyval}>
              <span className={styles.keyvalLabel}>CI</span>
              <span className={styles.keyvalValue}>
                <Badge status={normalizeCiState(patch.github.ci.state)} />
                {patch.github.ci.failure && (
                  <span className={styles.ciFailure}>
                    {patch.github.ci.failure.name}
                  </span>
                )}
              </span>
            </div>
          )}
        </div>

        {/* Description */}
        {patch.description && (
          <div className={styles.section}>
            <span className={styles.sectionLabel}>Description</span>
            <div className={styles.prose}>
              <Markdown content={patch.description} />
            </div>
          </div>
        )}

        {/* Tabs */}
        <div className={styles.tabs} role="tablist">
          {TABS.map((t) => (
            <button
              key={t.key}
              type="button"
              role="tab"
              className={`${styles.tab}${activeTab === t.key ? ` ${styles.tabActive}` : ""}`}
              aria-selected={activeTab === t.key}
              onClick={() => setActiveTab(t.key)}
              data-testid={`patch-tab-${t.key}`}
            >
              {t.label}
            </button>
          ))}
        </div>

        <div className={styles.tabContent}>
          {activeTab === "diff" && <DiffViewer diff={patch.diff} />}
          {activeTab === "reviews" && <ReviewsList reviews={patch.reviews} />}
          {activeTab === "activity" && <PatchActivity patchId={record.patch_id} />}
        </div>
      </div>
    </div>
  );
}

interface ReviewsListProps {
  reviews: PatchVersionRecord["patch"]["reviews"];
}

function ReviewsList({ reviews }: ReviewsListProps) {
  if (reviews.length === 0) {
    return <p className={styles.reviewEmpty}>No reviews.</p>;
  }

  return (
    <ul className={styles.reviewList}>
      {reviews.map((review, i) => (
        <li key={i} className={styles.review}>
          <div className={styles.reviewHead}>
            <Avatar
              name={principalDisplayName(review.author)}
              kind={principalAvatarKind(review.author)}
              size="md"
            />
            <span className={styles.reviewAuthor}>
              {principalDisplayName(review.author)}
            </span>
            <Badge status={review.is_approved ? "approved" : "changes-requested"} />
            {review.submitted_at && (
              <span className={styles.reviewWhen}>{formatTimestamp(review.submitted_at)}</span>
            )}
          </div>
          {review.contents && (
            <div className={styles.reviewBody}>
              <Markdown content={review.contents} />
            </div>
          )}
        </li>
      ))}
    </ul>
  );
}
