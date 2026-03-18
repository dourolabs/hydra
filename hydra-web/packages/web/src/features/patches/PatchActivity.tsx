import { Badge } from "@hydra/ui";
import type { PatchVersionRecord } from "@hydra/api";
import { normalizePatchStatus } from "../../utils/statusMapping";
import { usePatchVersions } from "./usePatchVersions";
import { ActivityTimeline } from "../activity/ActivityTimeline";
import type { Change } from "../activity/types";
import styles from "../activity/ActivityTimeline.module.css";

interface PatchActivityProps {
  patchId: string;
}

function diffPatchVersions(
  prev: PatchVersionRecord,
  curr: PatchVersionRecord,
): Change[] {
  const changes: Change[] = [];
  const prevPatch = prev.patch;
  const currPatch = curr.patch;

  if (prevPatch.status !== currPatch.status) {
    changes.push({
      field: "status",
      before: prevPatch.status,
      after: currPatch.status,
    });
  }
  if (prevPatch.title !== currPatch.title) {
    changes.push({
      field: "title",
      before: prevPatch.title,
      after: currPatch.title,
    });
  }
  if (prevPatch.description !== currPatch.description) {
    changes.push({ field: "description" });
  }

  const prevReviewCount = prevPatch.reviews.length;
  const currReviewCount = currPatch.reviews.length;
  if (currReviewCount > prevReviewCount) {
    const newReviews = currPatch.reviews.slice(prevReviewCount);
    for (const review of newReviews) {
      changes.push({
        field: "review",
        after: `${review.author}: ${review.is_approved ? "approved" : "changes requested"}`,
      });
    }
  }

  if (prevPatch.branch_name !== currPatch.branch_name) {
    changes.push({
      field: "branch",
      before: prevPatch.branch_name ?? "none",
      after: currPatch.branch_name ?? "none",
    });
  }

  const prevPrUrl = prevPatch.github?.url;
  const currPrUrl = currPatch.github?.url;
  if (prevPrUrl !== currPrUrl && currPrUrl) {
    changes.push({ field: "github_pr", after: currPrUrl });
  }

  return changes;
}

function PatchChangeEntry({ change }: { change: Change }) {
  if (change.field === "status" && change.before && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Status</span>
        <span className={styles.statusTransition}>
          <Badge status={normalizePatchStatus(change.before)} />
          <span className={styles.arrow}>{"\u2192"}</span>
          <Badge status={normalizePatchStatus(change.after)} />
        </span>
      </div>
    );
  }

  if (change.field === "title") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Title</span>
        updated
      </div>
    );
  }

  if (change.field === "description") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Description</span>
        updated
      </div>
    );
  }

  if (change.field === "review" && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Review</span>
        {change.after}
      </div>
    );
  }

  if (change.field === "branch") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Branch</span>
        <span className={styles.statusTransition}>
          {change.before}
          <span className={styles.arrow}>{"\u2192"}</span>
          {change.after}
        </span>
      </div>
    );
  }

  if (change.field === "github_pr" && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>GitHub PR</span>
        linked
      </div>
    );
  }

  return null;
}

export function PatchActivity({ patchId }: PatchActivityProps) {
  const { data, isLoading } = usePatchVersions(patchId);
  const versions = data?.versions ?? [];

  return (
    <ActivityTimeline
      versions={versions}
      isLoading={isLoading}
      diffFn={diffPatchVersions}
      creationLabel="Patch created"
      renderChange={(change, i) => (
        <PatchChangeEntry key={i} change={change} />
      )}
    />
  );
}
