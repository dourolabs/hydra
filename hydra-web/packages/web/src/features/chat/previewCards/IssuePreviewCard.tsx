import { Avatar, Badge, TypeChip, type PreviewCardTone, type BadgeStatus } from "@hydra/ui";
import { useIssue } from "../../issues/useIssue";
import { principalAvatarKind, principalDisplayName } from "../../principal/formatPrincipal";
import { normalizeIssueStatus } from "../../../utils/statusMapping";
import { AgoTime } from "../../../components/Runtime/Runtime";
import {
  FallbackPreviewCard,
  MonoId,
  NavigatingPreviewCard,
  SkeletonPreviewCard,
} from "./cardHelpers";
import { KIND_LABEL, firstNonEmptyLine } from "./cardConstants";

const TONE_BY_STATUS: Partial<Record<BadgeStatus, PreviewCardTone>> = {
  open: "open",
  "in-progress": "in-progress",
  "issue-closed": "closed",
  closed: "closed",
  failed: "failed",
  dropped: "dropped",
  blocked: "blocked",
};

function toneForIssueStatus(status: string): PreviewCardTone {
  const normalized = normalizeIssueStatus(status);
  return TONE_BY_STATUS[normalized] ?? "neutral";
}

interface IssuePreviewCardProps {
  id: string;
}

export function IssuePreviewCard({ id }: IssuePreviewCardProps) {
  const { data, isLoading, isError } = useIssue(id);
  const to = `/issues/${id}`;

  if (isLoading) {
    return <SkeletonPreviewCard id={id} kindLabel={KIND_LABEL.issue} />;
  }
  if (isError || !data) {
    return <FallbackPreviewCard id={id} kindLabel={KIND_LABEL.issue} to={to} />;
  }

  const issue = data.issue;
  const tone = toneForIssueStatus(issue.status);
  const status = normalizeIssueStatus(issue.status);
  const excerpt = firstNonEmptyLine(issue.description);
  const assignee = issue.assignee ?? null;
  const assigneeName = assignee ? principalDisplayName(assignee) : null;
  const title = issue.title || id;

  return (
    <NavigatingPreviewCard
      to={to}
      tone={tone}
      ariaLabel={`Issue ${id}: ${title}`}
      topRow={
        <>
          <Badge status={status} />
          <MonoId id={id} />
        </>
      }
      title={title}
      bodyExcerpt={excerpt ?? undefined}
      footer={
        <>
          {issue.type && issue.type !== "unknown" && <TypeChip type={issue.type} />}
          {assignee && assigneeName && (
            <Avatar
              name={assigneeName}
              kind={principalAvatarKind(assignee)}
              size="sm"
              title={`Assignee · ${assigneeName}`}
            />
          )}
          <AgoTime iso={data.timestamp} />
        </>
      }
    />
  );
}
