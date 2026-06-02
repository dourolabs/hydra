import { Badge, type BadgeStatus, type PreviewCardTone } from "@hydra/ui";
import { useSession } from "../../sessions/useSession";
import { normalizeSessionStatus } from "../../../utils/statusMapping";
import { AgoTime } from "../../../components/Runtime/Runtime";
import {
  FallbackPreviewCard,
  MonoId,
  NavigatingPreviewCard,
  SkeletonPreviewCard,
} from "./cardHelpers";
import { KIND_LABEL } from "./cardConstants";
import styles from "./previewCards.module.css";

const TONE_BY_STATUS: Partial<Record<BadgeStatus, PreviewCardTone>> = {
  created: "open",
  pending: "open",
  running: "in-progress",
  complete: "closed",
  failed: "failed",
};

function toneForSessionStatus(status: BadgeStatus): PreviewCardTone {
  return TONE_BY_STATUS[status] ?? "neutral";
}

interface SessionPreviewCardProps {
  id: string;
}

export function SessionPreviewCard({ id }: SessionPreviewCardProps) {
  const { data, isLoading, isError } = useSession(id);
  const to = `/sessions/${id}`;

  if (isLoading) {
    return <SkeletonPreviewCard id={id} kindLabel={KIND_LABEL.session} />;
  }
  if (isError || !data) {
    return <FallbackPreviewCard id={id} kindLabel={KIND_LABEL.session} to={to} />;
  }

  const session = data.session;
  const status = normalizeSessionStatus(session.status);
  const tone = toneForSessionStatus(status);
  const agentName = session.agent_config.agent_name ?? null;
  const model = session.agent_config.model ?? null;
  const issueId = session.spawned_from ?? null;
  const titleText = agentName ? `${agentName} · session` : "session";

  const footerNodes: React.ReactNode[] = [];
  if (session.creation_time) {
    footerNodes.push(
      <span key="created">
        created <AgoTime iso={session.creation_time} />
      </span>,
    );
  }
  if (session.end_time) {
    footerNodes.push(
      <span key="ended">
        ended <AgoTime iso={session.end_time} />
      </span>,
    );
  }

  return (
    <NavigatingPreviewCard
      to={to}
      tone={tone}
      ariaLabel={`Session ${id}`}
      topRow={
        <>
          <Badge status={status} />
          <MonoId id={id} />
          <AgoTime iso={data.timestamp} />
        </>
      }
      title={titleText}
      bodyExcerpt={
        issueId || model ? (
          <>
            {issueId && (
              <span className={styles.sessionBodyLine} data-mono="true">
                {issueId}
              </span>
            )}
            {model && <span className={styles.sessionBodyLine}>{model}</span>}
          </>
        ) : undefined
      }
      footer={footerNodes.length > 0 ? <>{footerNodes}</> : undefined}
    />
  );
}
