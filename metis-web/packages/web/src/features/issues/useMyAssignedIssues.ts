import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";
import { useAuth } from "../auth/useAuth";
import { actorDisplayName } from "../../api/auth";

export function useMyAssignedIssues() {
  const { user } = useAuth();
  const username = user ? actorDisplayName(user.actor) : "";

  return useQuery({
    queryKey: ["issues", "assigned", username],
    queryFn: () => apiClient.listIssues({ assignee: username }),
    select: (data) =>
      data.issues.filter(
        (i) => i.issue.status !== "closed" && i.issue.status !== "dropped" && i.issue.status !== "rejected",
      ),
    enabled: !!username,
  });
}
