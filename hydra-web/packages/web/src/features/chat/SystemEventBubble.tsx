import { Link } from "react-router-dom";
import type { SystemEventKind } from "@hydra/api";
import { formatTimestamp } from "../../utils/time";
import { AgoTime } from "../../components/Runtime/Runtime";
import { useIssue } from "../issues/useIssue";
import { StatusChip } from "../projects/StatusChip";
import styles from "./SystemEventBubble.module.css";

interface SystemEventBubbleProps {
  kind: SystemEventKind;
  timestamp: string;
}

interface ChildUnblockedChipProps {
  childId: string;
  fallbackStatusKey: string;
}

function ChildUnblockedChip({ childId, fallbackStatusKey }: ChildUnblockedChipProps) {
  const { data: record } = useIssue(childId);
  const title = record?.issue.title ?? childId;
  const status = record?.issue.status ?? null;
  return (
    <Link
      to={`/issues/${childId}`}
      className={styles.chip}
      data-testid="system-event-child-unblocked-chip"
      data-child-id={childId}
    >
      <span className={styles.chipTitle}>{title}</span>
      {status ? (
        <StatusChip status={status} className={styles.chipStatus} />
      ) : (
        <span className={styles.chipStatusFallback}>{fallbackStatusKey}</span>
      )}
    </Link>
  );
}

export function SystemEventBubble({ kind, timestamp }: SystemEventBubbleProps) {
  const tooltip = formatTimestamp(timestamp);
  return (
    <div
      className={styles.row}
      data-testid="system-event-bubble"
      data-kind={kind.kind}
      title={tooltip}
    >
      <div className={styles.body}>
        <span className={styles.lead}>System event</span>
        {kind.kind === "child_unblocked" ? (
          <>
            <span className={styles.text}>child unblocked —</span>
            <ChildUnblockedChip
              childId={kind.child_id}
              fallbackStatusKey={kind.new_status}
            />
          </>
        ) : (
          // Forward-compat: future `SystemEventKind` variants render as a
          // generic line until a dedicated arm is added. The discriminator
          // string is opaque here on purpose — the backend's canonical
          // `render()` is Rust-only and not parity-tested on the wire.
          <span className={styles.text}>received from server</span>
        )}
      </div>
      <span className={styles.when} title={tooltip}>
        <AgoTime iso={timestamp} />
      </span>
    </div>
  );
}
