import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Textarea } from "@metis/ui";
import { apiClient } from "../api/client";
import { useAuth } from "../features/auth/useAuth";
import { useToast } from "../features/toast/useToast";
import { actorDisplayName } from "../api/auth";
import styles from "./QuickCreate.module.css";

export function QuickCreate() {
  const { user } = useAuth();
  const { addToast } = useToast();
  const currentUsername = user ? actorDisplayName(user.actor) : "";
  const [description, setDescription] = useState("");
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (desc: string) =>
      apiClient.createIssue({
        issue: {
          type: "task",
          description: desc,
          creator: currentUsername,
          progress: "",
          status: "open",
          dependencies: [],
          patches: [],
        },
        job_id: null,
      }),
    onSuccess: (data) => {
      setDescription("");
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      addToast(`Issue ${data.issue_id} created`, "success");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to create issue",
        "error",
      );
    },
  });

  const handleSubmit = () => {
    const desc = description.trim();
    if (!desc) return;
    mutation.mutate(desc);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      handleSubmit();
    }
  };

  return (
    <div className={styles.quickCreate}>
      <Textarea
        placeholder="Quick create issue..."
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        onKeyDown={handleKeyDown}
        rows={1}
        className={styles.input}
      />
      <Button
        variant="primary"
        size="sm"
        onClick={handleSubmit}
        disabled={!description.trim() || mutation.isPending}
      >
        {mutation.isPending ? "..." : "+"}
      </Button>
    </div>
  );
}
