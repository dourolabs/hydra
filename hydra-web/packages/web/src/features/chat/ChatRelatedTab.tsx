import { Spinner } from "@hydra/ui";
import {
  RelatedSection,
  RelatedEmpty,
  LoadMore,
} from "../related/RelatedSection";
import {
  IssueRailRow,
  PatchRailRow,
  DocumentRailRow,
} from "../related/RailRow";
import { usePageIssueTrees } from "../dashboard/usePageIssueTrees";
import { useChatReferencedArtifacts } from "./useChatReferencedArtifacts";
import styles from "./ChatRelatedTab.module.css";

interface ChatRelatedTabProps {
  conversationId: string;
}

export function ChatRelatedTab({ conversationId }: ChatRelatedTabProps) {
  const {
    issues,
    patches,
    documents,
    sessionsByIssue,
    isLoading,
    error,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
  } = useChatReferencedArtifacts(conversationId);

  // Hydrate per-card neighborhood data (direct blockers + direct children) so
  // FlowPills render here the same way they do on the issues list.
  const { neighborhoodMap } = usePageIssueTrees(issues);

  if (isLoading) {
    return (
      <div className={styles.relatedTab}>
        <div className={styles.spinnerWrapper}>
          <Spinner size="sm" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className={styles.relatedTab}>
        <p className={styles.error}>Failed to load referenced items.</p>
      </div>
    );
  }

  return (
    <div className={styles.relatedTab}>
      <RelatedSection title="Issues" count={issues.length}>
        {issues.length === 0 ? (
          <RelatedEmpty>No issues referenced by this chat yet.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {issues.map((record) => (
              <IssueRailRow
                key={record.issue_id}
                record={record}
                sessions={sessionsByIssue.get(record.issue_id)}
                neighborhood={neighborhoodMap.get(record.issue_id)}
              />
            ))}
          </div>
        )}
        {hasNextPage.issues && (
          <LoadMore
            isFetching={isFetchingNextPage.issues}
            onClick={fetchNextPage.issues}
          />
        )}
      </RelatedSection>

      <RelatedSection title="Patches" count={patches.length}>
        {patches.length === 0 ? (
          <RelatedEmpty>No patches referenced by this chat yet.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {patches.map((record) => (
              <PatchRailRow key={record.patch_id} record={record} />
            ))}
          </div>
        )}
        {hasNextPage.patches && (
          <LoadMore
            isFetching={isFetchingNextPage.patches}
            onClick={fetchNextPage.patches}
          />
        )}
      </RelatedSection>

      <RelatedSection title="Documents" count={documents.length}>
        {documents.length === 0 ? (
          <RelatedEmpty>No documents referenced by this chat yet.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {documents.map((record) => (
              <DocumentRailRow key={record.document_id} record={record} />
            ))}
          </div>
        )}
        {hasNextPage.documents && (
          <LoadMore
            isFetching={isFetchingNextPage.documents}
            onClick={fetchNextPage.documents}
          />
        )}
      </RelatedSection>
    </div>
  );
}
