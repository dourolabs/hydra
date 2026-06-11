import { Button } from "@hydra/ui";
import { useArchiveIssue } from "./useArchiveIssue";

interface ArchiveIssueButtonProps {
  issueId: string;
  className?: string;
  variant?: "ghost" | "secondary";
  "data-testid"?: string;
}

/**
 * Manual "Archive" action. Hits the same soft-delete endpoint as the
 * auto-archive worker (`DELETE /v1/issues/:id`). Cache surgery lives in
 * `useArchiveIssue` so the same mutation can drive a menu item too.
 */
export function ArchiveIssueButton({
  issueId,
  className,
  variant = "ghost",
  "data-testid": testId,
}: ArchiveIssueButtonProps) {
  const { archive, isPending } = useArchiveIssue(issueId);

  return (
    <Button
      type="button"
      variant={variant}
      size="sm"
      className={className}
      disabled={isPending}
      aria-label="Archive issue"
      data-testid={testId}
      onClick={(e) => {
        e.stopPropagation();
        e.preventDefault();
        if (!isPending) archive();
      }}
    >
      {isPending ? "Archiving…" : "Archive"}
    </Button>
  );
}
