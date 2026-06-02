import { useDocument } from "../../documents/useDocument";
import { AgoTime } from "../../../components/Runtime/Runtime";
import {
  FallbackPreviewCard,
  MonoId,
  NavigatingPreviewCard,
  SkeletonPreviewCard,
} from "./cardHelpers";
import { KIND_LABEL, firstNonEmptyLine } from "./cardConstants";
import styles from "./previewCards.module.css";

interface DocumentPreviewCardProps {
  id: string;
}

export function DocumentPreviewCard({ id }: DocumentPreviewCardProps) {
  const { data, isLoading, isError } = useDocument(id);
  const to = `/documents/${id}`;

  if (isLoading) {
    return <SkeletonPreviewCard id={id} kindLabel={KIND_LABEL.document} />;
  }
  if (isError || !data) {
    return <FallbackPreviewCard id={id} kindLabel={KIND_LABEL.document} to={to} />;
  }

  const doc = data.document;
  const title = doc.title || doc.path || id;
  const excerpt = firstNonEmptyLine(doc.body_markdown);
  const path = doc.path && doc.path !== title ? doc.path : null;

  return (
    <NavigatingPreviewCard
      to={to}
      tone="neutral"
      ariaLabel={`Document ${id}: ${title}`}
      topRow={
        <>
          <MonoId id={id} />
        </>
      }
      title={title}
      bodyExcerpt={excerpt ?? undefined}
      footer={
        <>
          <span className={styles.kindChip}>{KIND_LABEL.document}</span>
          {path && <span data-pc-mono="true">{path}</span>}
          <AgoTime iso={data.timestamp} />
        </>
      }
    />
  );
}
