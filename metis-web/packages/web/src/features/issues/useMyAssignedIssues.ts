import { useMemo } from "react";
import { useIssues } from "./useIssues";
import { useAuth } from "../auth/useAuth";
import { actorDisplayName } from "../../api/auth";

export function useMyAssignedIssues() {
  const { user } = useAuth();
  const { data: issues, isLoading, error } = useIssues();

  const currentUsername = user ? actorDisplayName(user.actor) : null;

  const assignedIssues = useMemo(() => {
    if (!issues || !currentUsername) return [];
    return issues.filter(
      (record) =>
        record.issue.assignee === currentUsername &&
        record.issue.status !== "closed" &&
        record.issue.status !== "dropped" &&
        record.issue.status !== "rejected",
    );
  }, [issues, currentUsername]);

  return { data: assignedIssues, isLoading, error };
}
