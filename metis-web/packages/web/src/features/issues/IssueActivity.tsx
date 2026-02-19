import { useQuery } from "@tanstack/react-query";
import { Avatar, Badge, Spinner, type BadgeStatus } from "@metis/ui";
import {
  fetchIssueVersions,
  type ActorRef,
  type IssueData,
  type IssueVersionRecord,
} from "../../api/issues";
import styles from "./IssueActivity.module.css";

interface IssueActivityProps {
  issueId: string;
}

const validStatuses: Set<string> = new Set([
  "open",
  "in-progress",
  "closed",
  "failed",
  "dropped",
  "blocked",
  "rejected",
]);

function toBadgeStatus(status: string): BadgeStatus {
  if (validStatuses.has(status)) return status as BadgeStatus;
  return "open";
}

/** Extract a human-readable display name from an ActorRef. */
function actorDisplayName(actor: ActorRef): string {
  if ("Authenticated" in actor) {
    const id = actor.Authenticated.actor_id;
    if ("Username" in id) return id.Username;
    return id.Task;
  }
  if ("System" in actor) {
    const { worker_name, on_behalf_of } = actor.System;
    if (on_behalf_of) {
      const name = "Username" in on_behalf_of ? on_behalf_of.Username : on_behalf_of.Task;
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

/** Determine the short name used for the Avatar component. */
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

/** Diff two adjacent issue versions and return a list of what changed. */
function diffVersions(prev: IssueData, curr: IssueData): Change[] {
  const changes: Change[] = [];

  if (prev.status !== curr.status) {
    changes.push({ field: "status", before: prev.status, after: curr.status });
  }
  if (prev.assignee !== curr.assignee) {
    changes.push({
      field: "assignee",
      before: prev.assignee ?? "unassigned",
      after: curr.assignee ?? "unassigned",
    });
  }
  if (prev.progress !== curr.progress) {
    changes.push({ field: "progress" });
  }
  if (prev.description !== curr.description) {
    changes.push({ field: "description" });
  }
  if (prev.type !== curr.type) {
    changes.push({ field: "type", before: prev.type, after: curr.type });
  }

  // Detect patch list changes
  const prevPatches = new Set(prev.patches);
  const currPatches = new Set(curr.patches);
  for (const p of currPatches) {
    if (!prevPatches.has(p)) {
      changes.push({ field: "patch", after: p });
    }
  }

  // Detect dependency changes
  const prevDeps = JSON.stringify(prev.dependencies);
  const currDeps = JSON.stringify(curr.dependencies);
  if (prevDeps !== currDeps) {
    changes.push({ field: "dependencies" });
  }

  return changes;
}

function formatTimestamp(ts: string): string {
  const date = new Date(ts);
  return date.toLocaleString();
}

interface TimelineEntryProps {
  version: IssueVersionRecord;
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
          <span className={styles.version}>v{version.version}</span>
        </div>

        <div className={styles.changes}>
          {isCreation && (
            <span className={styles.created}>Issue created</span>
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
          <Badge status={toBadgeStatus(change.before)} />
          <span className={styles.arrow}>{"\u2192"}</span>
          <Badge status={toBadgeStatus(change.after)} />
        </span>
      </div>
    );
  }

  if (change.field === "assignee") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Assignee</span>
        <span className={styles.statusTransition}>
          {change.before ?? "unassigned"}
          <span className={styles.arrow}>{"\u2192"}</span>
          {change.after ?? "unassigned"}
        </span>
      </div>
    );
  }

  if (change.field === "progress") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Progress</span>
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

  if (change.field === "type" && change.before && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Type</span>
        <span className={styles.statusTransition}>
          {change.before}
          <span className={styles.arrow}>{"\u2192"}</span>
          {change.after}
        </span>
      </div>
    );
  }

  if (change.field === "patch" && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Patch</span>
        {change.after} linked
      </div>
    );
  }

  if (change.field === "dependencies") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Dependencies</span>
        updated
      </div>
    );
  }

  return null;
}

export function IssueActivity({ issueId }: IssueActivityProps) {
  const { data, isLoading } = useQuery({
    queryKey: ["issue", issueId, "versions"],
    queryFn: () => fetchIssueVersions(issueId),
  });

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  const versions = data?.versions ?? [];

  if (versions.length === 0) {
    return <p className={styles.empty}>No activity.</p>;
  }

  // Sort newest-first for display
  const sorted = [...versions].sort((a, b) => b.version - a.version);

  // Build timeline entries by diffing adjacent versions
  // Versions from the API are ordered by version number ascending
  const byVersion = [...versions].sort((a, b) => a.version - b.version);

  type EntryData = {
    version: IssueVersionRecord;
    changes: Change[];
    isCreation: boolean;
  };

  const entries: EntryData[] = sorted.map((v) => {
    const idx = byVersion.findIndex((bv) => bv.version === v.version);
    if (idx === 0) {
      // First version — this is the creation event
      return { version: v, changes: [], isCreation: true };
    }
    const prev = byVersion[idx - 1];
    return {
      version: v,
      changes: diffVersions(prev.issue, v.issue),
      isCreation: false,
    };
  });

  return (
    <div className={styles.container}>
      <ul className={styles.timeline}>
        {entries.map((entry) => (
          <TimelineEntry
            key={entry.version.version}
            version={entry.version}
            changes={entry.changes}
            isCreation={entry.isCreation}
          />
        ))}
      </ul>
    </div>
  );
}
