import { Link } from "react-router-dom";
import { Avatar, Badge, DiffViewer, Spinner } from "@hydra/ui";
import { normalizePatchStatus } from "../../utils/statusMapping";
import { usePatch } from "../patches/usePatch";
import styles from "./PatchPreview.module.css";

interface PatchPreviewCardProps {
  patchId: string;
  issueId: string;
}

function PatchPreviewCard({ patchId, issueId }: PatchPreviewCardProps) {
  const { data: record, isLoading, error } = usePatch(patchId);

  if (isLoading) {
    return (
      <div className={styles.patchCard}>
        <Spinner size="sm" />
      </div>
    );
  }

  if (error || !record) {
    return (
      <div className={styles.patchCard}>
        <p className={styles.error}>
          Failed to load patch {patchId}
        </p>
      </div>
    );
  }

  const { patch } = record;

  return (
    <div className={styles.patchCard}>
      <div className={styles.patchHeader}>
        <Link
          to={`/patches/${record.patch_id}?issueId=${issueId}`}
          className={styles.patchIdLink}
        >
          {record.patch_id}
        </Link>
        <Badge status={normalizePatchStatus(patch.status)} />
      </div>

      <p className={styles.patchTitle}>{patch.title}</p>

      {patch.github?.url && (
        <a
          href={patch.github.url}
          target="_blank"
          rel="noopener noreferrer"
          className={styles.ghLink}
        >
          {patch.github.owner}/{patch.github.repo}#{String(patch.github.number)}{" "}
          ↗
        </a>
      )}

      {patch.reviews.length > 0 && (
        <div className={styles.patchReviews}>
          {patch.reviews.map((review, i) => (
            <span key={i} className={styles.patchReviewChip}>
              <Avatar name={review.author} size="sm" />
              {review.author}
              {" \u2014 "}
              <Badge
                status={review.is_approved ? "approved" : "changes-requested"}
              />
            </span>
          ))}
        </div>
      )}

      {patch.diff && (
        <DiffViewer
          diff={patch.diff}
          maxLines={200}
          className={styles.diffViewer}
        />
      )}
    </div>
  );
}

interface PatchPreviewProps {
  patchIds: string[];
  issueId: string;
}

export function PatchPreview({ patchIds, issueId }: PatchPreviewProps) {
  if (patchIds.length === 0) {
    return <p className={styles.empty}>No patches.</p>;
  }

  return (
    <div className={styles.container}>
      {patchIds.map((patchId) => (
        <PatchPreviewCard
          key={patchId}
          patchId={patchId}
          issueId={issueId}
        />
      ))}
    </div>
  );
}
