import { Badge, type BadgeStatus, type PreviewCardTone } from "@hydra/ui";
import { useConversation } from "../useConversations";
import { CONVERSATION_STATUS_TONES } from "../conversationStatusBadge";
import { AgoTime } from "../../../components/Runtime/Runtime";
import {
  FallbackPreviewCard,
  MonoId,
  NavigatingPreviewCard,
  SkeletonPreviewCard,
} from "./cardHelpers";
import { KIND_LABEL } from "./cardConstants";
import styles from "./previewCards.module.css";

const TONE_BY_STATUS: Record<BadgeStatus, PreviewCardTone> = {
  open: "open",
  "in-progress": "in-progress",
  closed: "closed",
  "issue-closed": "closed",
  failed: "failed",
  dropped: "dropped",
  blocked: "blocked",
  merged: "closed",
  "changes-requested": "rejected",
  approved: "closed",
  created: "open",
  pending: "open",
  running: "in-progress",
  complete: "closed",
  success: "closed",
  "conv-active": "in-progress",
  "conv-idle": "open",
  "conv-closed": "closed",
  archived: "neutral",
  unknown: "neutral",
};

interface ConversationPreviewCardProps {
  id: string;
}

export function ConversationPreviewCard({ id }: ConversationPreviewCardProps) {
  const { data, isLoading, isError } = useConversation(id);
  const to = `/chat/${id}`;

  if (isLoading) {
    return <SkeletonPreviewCard id={id} kindLabel={KIND_LABEL.conversation} />;
  }
  if (isError || !data) {
    return <FallbackPreviewCard id={id} kindLabel={KIND_LABEL.conversation} to={to} />;
  }

  const status: BadgeStatus = CONVERSATION_STATUS_TONES[data.status];
  const tone = TONE_BY_STATUS[status] ?? "neutral";
  const title = data.title || id;
  const agent = data.agent_name ?? null;

  return (
    <NavigatingPreviewCard
      to={to}
      tone={tone}
      ariaLabel={`Conversation ${id}: ${title}`}
      topRow={
        <>
          <Badge status={status} />
          <MonoId id={id} />
        </>
      }
      title={title}
      footer={
        <>
          <span className={styles.kindChip}>{KIND_LABEL.conversation}</span>
          {agent && <span className={styles.assigneeName}>{agent}</span>}
          <AgoTime iso={data.updated_at} />
        </>
      }
    />
  );
}
