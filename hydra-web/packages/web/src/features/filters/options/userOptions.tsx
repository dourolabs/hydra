import { useMemo } from "react";
import { Avatar } from "@hydra/ui";
import { useAgents } from "../../../hooks/useAgents";
import { useUsers } from "../../../hooks/useUsers";
import type { FilterOption } from "../types";
import styles from "./userOptions.module.css";

export interface UserOption extends FilterOption {
  kind: "agent" | "human";
}

/**
 * Returns the union of agent and human users as `FilterOption[]`.
 *
 * `value` is the canonical Principal path (`agents/<name>` / `users/<name>`)
 * so callers can drive filters whose values match the wire form used by
 * `assignee` and similar fields. The chip / row render an `<Avatar>` keyed
 * to the kind, and `sub` (`"agent"` / `"human"`) is plumbed through so the
 * value-picker's search can distinguish identical bare names.
 *
 * This is a hook (not a plain function) because it pulls live data from the
 * React Query cache via `useAgents` / `useUsers`. Memoised so repeated renders
 * don't re-create the option array (and the chips inside it).
 */
export function useUserOptions(): UserOption[] {
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  return useMemo(() => {
    const out: UserOption[] = [];
    const seen = new Set<string>();

    for (const agent of agents ?? []) {
      const value = `agents/${agent.name}`;
      if (seen.has(value)) continue;
      seen.add(value);
      out.push({
        value,
        label: agent.name,
        sub: "agent",
        kind: "agent",
        chip: <Avatar name={agent.name} kind="agent" size="sm" />,
        render: (
          <span className={styles.userRow}>
            <Avatar name={agent.name} kind="agent" size="md" />
            <span>{agent.name}</span>
          </span>
        ),
      });
    }

    for (const user of users ?? []) {
      const value = `users/${user.username}`;
      if (seen.has(value)) continue;
      seen.add(value);
      out.push({
        value,
        label: user.username,
        sub: "human",
        kind: "human",
        chip: <Avatar name={user.username} kind="human" size="sm" />,
        render: (
          <span className={styles.userRow}>
            <Avatar name={user.username} kind="human" size="md" />
            <span>{user.username}</span>
          </span>
        ),
      });
    }

    return out;
  }, [agents, users]);
}
