import { useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Picker, PickerRow } from "@hydra/ui";
import type { Issue, IssueVersionRecord, Principal } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useAgents } from "../../hooks/useAgents";
import { useUsers } from "../../hooks/useUsers";
import { useToast } from "../toast/useToast";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../principal/formatPrincipal";
import styles from "./IssueAssigneePicker.module.css";

interface IssueAssigneePickerProps {
  issueId: string;
  issue: Issue;
  /**
   * Hide the visual "Assignee" caption above the trigger pill. The label
   * is still wired through to the trigger's `aria-label`.
   */
  hideLabel?: boolean;
}

function principalsEqual(a: Principal | null, b: Principal | null): boolean {
  if (a === null || b === null) return a === b;
  if ("User" in a && "User" in b) return a.User.name === b.User.name;
  if ("Agent" in a && "Agent" in b) return a.Agent.name === b.Agent.name;
  return false;
}

export function IssueAssigneePicker({
  issueId,
  issue,
  hideLabel,
}: IssueAssigneePickerProps) {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();

  const agentNames = useMemo(
    () => Array.from(new Set((agents ?? []).map((a) => a.name))).sort(),
    [agents],
  );
  const userNames = useMemo(
    () => Array.from(new Set((users ?? []).map((u) => u.username))).sort(),
    [users],
  );

  const current = issue.assignee ?? null;

  const mutation = useMutation<unknown, Error, Principal | null, { previous?: IssueVersionRecord }>({
    mutationFn: (assignee) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: issue.status.key,
          assignee,
        },
        session_id: null,
      }),
    onMutate: async (assignee) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>(["issue", issueId]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: { ...previous.issue, assignee },
        });
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(err.message || "Failed to update assignee", "error");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const choose = (next: Principal | null) => {
    setOpen(false);
    if (principalsEqual(current, next)) return;
    mutation.mutate(next);
  };

  return (
    <Picker
      label="Assignee"
      hideLabel={hideLabel}
      open={open}
      onToggle={() => setOpen((v) => !v)}
      wide
      data-testid="issue-assignee-picker"
      value={
        current ? (
          <span className={styles.pillContent}>
            <Avatar
              name={principalDisplayName(current)}
              kind={principalAvatarKind(current)}
              size="sm"
            />
            <span>{principalDisplayName(current)}</span>
          </span>
        ) : (
          <span className={styles.pillEmpty}>Unassigned</span>
        )
      }
    >
      <PickerRow
        active={!current}
        onClick={() => choose(null)}
        data-testid="issue-assignee-option-unassigned"
      >
        <span className={styles.pillEmpty}>Unassigned</span>
        <span className={styles.popSpacer} />
      </PickerRow>
      {agentNames.length > 0 && (
        <>
          <div className={styles.popSection}>Agents</div>
          {agentNames.map((name) => {
            const isActive =
              !!current && "Agent" in current && current.Agent.name === name;
            return (
              <PickerRow
                key={`agents/${name}`}
                active={isActive}
                onClick={() => choose({ Agent: { name } })}
                data-testid={`issue-assignee-option-agent-${name}`}
              >
                <Avatar name={name} kind="agent" size="md" />
                <span>{name}</span>
                <span className={styles.popSpacer} />
              </PickerRow>
            );
          })}
        </>
      )}
      {userNames.length > 0 && (
        <>
          <div className={styles.popSection}>Users</div>
          {userNames.map((name) => {
            const isActive =
              !!current && "User" in current && current.User.name === name;
            return (
              <PickerRow
                key={`users/${name}`}
                active={isActive}
                onClick={() => choose({ User: { name } })}
                data-testid={`issue-assignee-option-user-${name}`}
              >
                <Avatar name={name} kind="human" size="md" />
                <span>{name}</span>
                <span className={styles.popSpacer} />
              </PickerRow>
            );
          })}
        </>
      )}
    </Picker>
  );
}
