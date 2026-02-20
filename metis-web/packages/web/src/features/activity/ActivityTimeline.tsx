import type { ReactNode } from "react";
import { Avatar, Spinner } from "@metis/ui";
import type { ActorRef } from "@metis/api";
import { actorDisplayName, actorAvatarName } from "../../utils/actors";
import { formatTimestamp } from "../../utils/time";
import styles from "./ActivityTimeline.module.css";

import type { Change } from "./types";

interface VersionLike {
  version: bigint;
  timestamp: string;
  actor?: ActorRef | null;
}

interface TimelineEntryProps {
  version: VersionLike;
  changes: Change[];
  isCreation: boolean;
  creationLabel: string;
  renderChange: (change: Change, index: number) => ReactNode;
}

export function TimelineEntry({
  version,
  changes,
  isCreation,
  creationLabel,
  renderChange,
}: TimelineEntryProps) {
  const actor = version.actor;

  return (
    <li className={styles.entry}>
      <div className={styles.entryContent}>
        <div className={styles.entryHeader}>
          {actor && (
            <span className={styles.actor}>
              <Avatar name={actorAvatarName(actor)} size="sm" />
              {actorDisplayName(actor)}
            </span>
          )}
          <span className={styles.timestamp}>
            {formatTimestamp(version.timestamp)}
          </span>
          <span className={styles.version}>v{String(version.version)}</span>
        </div>

        <div className={styles.changes}>
          {isCreation && (
            <span className={styles.created}>{creationLabel}</span>
          )}
          {changes.map((change, i) => renderChange(change, i))}
        </div>
      </div>
    </li>
  );
}

interface EntryData<V extends VersionLike> {
  version: V;
  changes: Change[];
  isCreation: boolean;
}

interface ActivityTimelineProps<V extends VersionLike> {
  versions: V[];
  isLoading: boolean;
  diffFn: (prev: V, curr: V) => Change[];
  creationLabel: string;
  renderChange: (change: Change, index: number) => ReactNode;
}

export function ActivityTimeline<V extends VersionLike>({
  versions,
  isLoading,
  diffFn,
  creationLabel,
  renderChange,
}: ActivityTimelineProps<V>) {
  if (isLoading) {
    return <Spinner size="sm" />;
  }

  if (versions.length === 0) {
    return <p className={styles.empty}>No activity.</p>;
  }

  const sorted = [...versions].sort((a, b) =>
    a.version > b.version ? -1 : a.version < b.version ? 1 : 0,
  );

  const byVersion = [...versions].sort((a, b) =>
    a.version < b.version ? -1 : a.version > b.version ? 1 : 0,
  );

  const entries: EntryData<V>[] = sorted.map((v) => {
    const idx = byVersion.findIndex((bv) => bv.version === v.version);
    if (idx === 0) {
      return { version: v, changes: [], isCreation: true };
    }
    const prev = byVersion[idx - 1];
    return {
      version: v,
      changes: diffFn(prev, v),
      isCreation: false,
    };
  });

  return (
    <div className={styles.container}>
      <ul className={styles.timeline}>
        {entries.map((entry) => (
          <TimelineEntry
            key={String(entry.version.version)}
            version={entry.version}
            changes={entry.changes}
            isCreation={entry.isCreation}
            creationLabel={creationLabel}
            renderChange={renderChange}
          />
        ))}
      </ul>
    </div>
  );
}
