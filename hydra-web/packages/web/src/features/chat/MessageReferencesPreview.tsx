import { hydraIdKind, type HydraIdKind } from "@hydra/api";
import { extractHydraReferences } from "./extractHydraReferences";
import { IssuePreviewCard } from "./previewCards/IssuePreviewCard";
import { PatchPreviewCard } from "./previewCards/PatchPreviewCard";
import { DocumentPreviewCard } from "./previewCards/DocumentPreviewCard";
import { SessionPreviewCard } from "./previewCards/SessionPreviewCard";
import { ConversationPreviewCard } from "./previewCards/ConversationPreviewCard";
import styles from "./MessageReferencesPreview.module.css";

const SUPPORTED: ReadonlySet<HydraIdKind> = new Set([
  "issue",
  "patch",
  "document",
  "session",
  "conversation",
]);

interface MessageReferencesPreviewProps {
  /** Raw message body. References are extracted from text, skipping code spans. */
  content: string;
}

function CardForId({ id }: { id: string }) {
  const kind = hydraIdKind(id);
  switch (kind) {
    case "issue":
      return <IssuePreviewCard id={id} />;
    case "patch":
      return <PatchPreviewCard id={id} />;
    case "document":
      return <DocumentPreviewCard id={id} />;
    case "session":
      return <SessionPreviewCard id={id} />;
    case "conversation":
      return <ConversationPreviewCard id={id} />;
    default:
      return null;
  }
}

/**
 * Stack a preview card for each unique Hydra reference in `content`, in
 * source order. Renders nothing when no supported references are present.
 */
export function MessageReferencesPreview({ content }: MessageReferencesPreviewProps) {
  const ids = extractHydraReferences(content).filter((id) => {
    const kind = hydraIdKind(id);
    return kind !== null && SUPPORTED.has(kind);
  });
  if (ids.length === 0) return null;

  return (
    <div className={styles.stack} data-testid="message-references-preview">
      {ids.map((id) => (
        <CardForId key={id} id={id} />
      ))}
    </div>
  );
}
