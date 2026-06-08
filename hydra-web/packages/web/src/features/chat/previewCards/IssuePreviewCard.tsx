import { Avatar, TypeChip, type PreviewCardTone } from "@hydra/ui";
import { useIssue } from "../../issues/useIssue";
import { principalAvatarKind, principalDisplayName } from "../../principal/formatPrincipal";
import { ProjectChip } from "../../projects/ProjectChip";
import { StatusChip } from "../../projects/StatusChip";
import { useProjects } from "../../projects/useProjects";
import { AgoTime } from "../../../components/Runtime/Runtime";
import {
  FallbackPreviewCard,
  MonoId,
  NavigatingPreviewCard,
  SkeletonPreviewCard,
} from "./cardHelpers";
import { KIND_LABEL, firstNonEmptyLine } from "./cardConstants";

const TONE_BY_STATUS_KEY: Record<string, PreviewCardTone> = {
  open: "open",
  "in-progress": "in-progress",
  closed: "closed",
  failed: "failed",
  dropped: "dropped",
};

function toneForIssueStatus(status: string): PreviewCardTone {
  return TONE_BY_STATUS_KEY[status] ?? "neutral";
}

interface IssuePreviewCardProps {
  id: string;
}

export function IssuePreviewCard({ id }: IssuePreviewCardProps) {
  const { data, isLoading, isError } = useIssue(id);
  const { data: projects } = useProjects();
  const to = `/issues/${id}`;

  if (isLoading) {
    return <SkeletonPreviewCard id={id} kindLabel={KIND_LABEL.issue} />;
  }
  if (isError || !data) {
    return <FallbackPreviewCard id={id} kindLabel={KIND_LABEL.issue} to={to} />;
  }

  const issue = data.issue;
  const tone = toneForIssueStatus(issue.status);
  const excerpt = firstNonEmptyLine(issue.description);
  const assignee = issue.assignee ?? null;
  const assigneeName = assignee ? principalDisplayName(assignee) : null;
  const title = issue.title || id;
  const projectKey = issue.project_id
    ? projects?.find((p) => p.project_id === issue.project_id)?.project.key ?? null
    : projects?.find((p) => p.project.key === "default")?.project.key ?? null;

  return (
    <NavigatingPreviewCard
      to={to}
      tone={tone}
      ariaLabel={`Issue ${id}: ${title}`}
      topRow={
        <>
          <StatusChip definition={issue.resolved_status} fallbackKey={issue.status} />
          {projectKey && (
            <ProjectChip
              projectKey={projectKey}
              data-testid={`issue-preview-project-chip-${id}`}
            />
          )}
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
