import type { MergePolicy, ReviewerGroup } from "@hydra/api";
import styles from "./MergePolicySummary.module.css";

interface MergePolicySummaryProps {
  policy: MergePolicy | null | undefined;
  repoName: string;
}

export function MergePolicySummary({ policy, repoName }: MergePolicySummaryProps) {
  if (!policy) {
    return (
      <span className={styles.dash} data-testid={`merge-policy-${repoName}-none`}>
        —
      </span>
    );
  }

  return (
    <div className={styles.policy} data-testid={`merge-policy-${repoName}`}>
      <ReviewersBlock reviewers={policy.reviewers} repoName={repoName} />
      <MergersBlock mergers={policy.mergers ?? null} repoName={repoName} />
    </div>
  );
}

function ReviewersBlock({
  reviewers,
  repoName,
}: {
  reviewers: ReviewerGroup[];
  repoName: string;
}) {
  return (
    <div className={styles.block}>
      <span className={styles.label}>Reviewers</span>
      {reviewers.length === 0 ? (
        <span className={styles.unset} data-testid={`merge-policy-${repoName}-reviewers-none`}>
          none required
        </span>
      ) : (
        <div className={styles.groups}>
          {reviewers.map((group, idx) => (
            <ReviewerGroupRow key={idx} group={group} />
          ))}
        </div>
      )}
    </div>
  );
}

function ReviewerGroupRow({ group }: { group: ReviewerGroup }) {
  const count = group.count ?? 1;
  // exclude_author defaults to true on the wire; mirror that here for display.
  const excludeAuthor = group.exclude_author ?? true;
  const meta: string[] = [];
  if (count > 1) meta.push(`${count} required`);
  if (!excludeAuthor) meta.push("author may approve");

  return (
    <div className={styles.group}>
      {group.label ? <span className={styles.groupLabel}>{group.label}:</span> : null}
      <span className={styles.principals}>
        {group.any_of.map((p, i) => (
          <span key={i} className={styles.principal}>
            {p}
          </span>
        ))}
      </span>
      {meta.length > 0 ? <span className={styles.meta}>({meta.join("; ")})</span> : null}
    </div>
  );
}

function MergersBlock({
  mergers,
  repoName,
}: {
  mergers: MergePolicy["mergers"] | null;
  repoName: string;
}) {
  return (
    <div className={styles.block}>
      <span className={styles.label}>Mergers</span>
      {mergers ? (
        <span className={styles.principals}>
          {mergers.any_of.map((p, i) => (
            <span key={i} className={styles.principal}>
              {p}
            </span>
          ))}
        </span>
      ) : (
        <span className={styles.unset} data-testid={`merge-policy-${repoName}-mergers-unset`}>
          unset (any approver)
        </span>
      )}
    </div>
  );
}
