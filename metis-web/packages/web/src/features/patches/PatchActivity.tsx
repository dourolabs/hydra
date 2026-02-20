import { Avatar, Badge, Spinner } from "@metis/ui";
import type { ActorRef, Patch, PatchVersionRecord } from "@metis/api";
import { patchToBadgeStatus } from "../../utils/statusMapping";
import { usePatchVersions } from "./usePatchVersions";
import styles from "./PatchActivity.module.css";

interface PatchActivityProps {
  patchId: string;
}

function actorDisplayName(actor: ActorRef): string {
  if ("Authenticated" in actor) {
    const id = actor.Authenticated.actor_id;
    if ("Username" in id) return id.Username;
    return id.Task;
  }
  if ("System" in actor) {
    const { worker_name, on_behalf_of } = actor.System;
    if (on_behalf_of) {
      const name =
        "Username" in on_behalf_of
          ? on_behalf_of.Username
          : on_behalf_of.Task;
      return `${worker_name} (on behalf of ${name})`;
    }
    return worker_name;
  }
  if ("Automation" in actor) {
    const { automation_name, triggered_by } = actor.Automation;
    if (triggered_by) {
      return `${automation_name} (triggered by ${actorDisplayName(triggered_by)})`;
    }
    return automation_name;
  }
  return "unknown";
}

function actorAvatarName(actor: ActorRef): string {
  if ("Authenticated" in actor) {
    const id = actor.Authenticated.actor_id;
    if ("Username" in id) return id.Username;
    return id.Task;
  }
  if ("System" in actor) return actor.System.worker_name;
  if ("Automation" in actor) return actor.Automation.automation_name;
  return "?";
}

interface Change {
  field: string;
  before?: string;
  after?: string;
}

function diffPatchVersions(prev: Patch, curr: Patch): Change[] {
  const changes: Change[] = [];

  if (prev.status !== curr.status) {
    changes.push({ field: "status", before: prev.status, after: curr.status });
  }
  if (prev.title !== curr.title) {
    changes.push({ field: "title", before: prev.title, after: curr.title });
  }
  if (prev.description !== curr.description) {
    changes.push({ field: "description" });
  }

  const prevReviewCount = prev.reviews.length;
  const currReviewCount = curr.reviews.length;
  if (currReviewCount > prevReviewCount) {
    const newReviews = curr.reviews.slice(prevReviewCount);
    for (const review of newReviews) {
      changes.push({
        field: "review",
        after: `${review.author}: ${review.is_approved ? "approved" : "changes requested"}`,
      });
    }
  }

  if (prev.branch_name !== curr.branch_name) {
    changes.push({
      field: "branch",
      before: prev.branch_name ?? "none",
      after: curr.branch_name ?? "none",
    });
  }

  const prevPrUrl = prev.github?.url;
  const currPrUrl = curr.github?.url;
  if (prevPrUrl !== currPrUrl && currPrUrl) {
    changes.push({ field: "github_pr", after: currPrUrl });
  }

  return changes;
}

function formatTimestamp(ts: string): string {
  return new Date(ts).toLocaleString();
}

interface TimelineEntryProps {
  version: PatchVersionRecord;
  changes: Change[];
  isCreation: boolean;
}

function TimelineEntry({ version, changes, isCreation }: TimelineEntryProps) {
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
            <span className={styles.created}>Patch created</span>
          )}
          {changes.map((change, i) => (
            <ChangeEntry key={i} change={change} />
          ))}
        </div>
      </div>
    </li>
  );
}

function ChangeEntry({ change }: { change: Change }) {
  if (change.field === "status" && change.before && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Status</span>
        <span className={styles.statusTransition}>
          <Badge status={patchToBadgeStatus(change.before)} />
          <span className={styles.arrow}>{"\u2192"}</span>
          <Badge status={patchToBadgeStatus(change.after)} />
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

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  const versions = data?.versions ?? [];

  if (versions.length === 0) {
    return <p className={styles.empty}>No activity.</p>;
  }

  const sorted = [...versions].sort((a, b) =>
    a.version > b.version ? -1 : a.version < b.version ? 1 : 0,
  );

  const byVersion = [...versions].sort((a, b) =>
    a.version < b.version ? -1 : a.version > b.version ? 1 : 0,
  );

  type EntryData = {
    version: PatchVersionRecord;
    changes: Change[];
    isCreation: boolean;
  };

  const entries: EntryData[] = sorted.map((v) => {
    const idx = byVersion.findIndex((bv) => bv.version === v.version);
    if (idx === 0) {
      return { version: v, changes: [], isCreation: true };
    }
    const prev = byVersion[idx - 1];
    return {
      version: v,
      changes: diffPatchVersions(prev.patch, v.patch),
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
          />
        ))}
      </ul>
    </div>
  );
}
