import { Avatar, Badge, type BadgeStatus, type PreviewCardTone } from "@hydra/ui";
import { usePatch } from "../../patches/usePatch";
import { normalizePatchStatus } from "../../../utils/statusMapping";
import { AgoTime } from "../../../components/Runtime/Runtime";
import {
  FallbackPreviewCard,
  MonoId,
  NavigatingPreviewCard,
  SkeletonPreviewCard,
} from "./cardHelpers";
import { KIND_LABEL, firstNonEmptyLine } from "./cardConstants";
import styles from "./previewCards.module.css";

const TONE_BY_STATUS: Partial<Record<BadgeStatus, PreviewCardTone>> = {
  open: "open",
  merged: "closed",
  closed: "failed",
  "changes-requested": "rejected",
  approved: "closed",
};

function toneForPatchStatus(status: BadgeStatus): PreviewCardTone {
  return TONE_BY_STATUS[status] ?? "neutral";
}

interface PatchPreviewCardProps {
  id: string;
}

export function PatchPreviewCard({ id }: PatchPreviewCardProps) {
  const { data, isLoading, isError } = usePatch(id);
  const to = `/patches/${id}`;

  if (isLoading) {
    return <SkeletonPreviewCard id={id} kindLabel={KIND_LABEL.patch} />;
  }
  if (isError || !data) {
    return <FallbackPreviewCard id={id} kindLabel={KIND_LABEL.patch} to={to} />;
  }

  const patch = data.patch;
  const status = normalizePatchStatus(patch.status);
  const tone = toneForPatchStatus(status);
  const excerpt = firstNonEmptyLine(patch.description);
  const title = patch.title || id;
  const repoName = patch.service_repo_name || null;
  const author = patch.creator || null;

  return (
    <NavigatingPreviewCard
      to={to}
      tone={tone}
      ariaLabel={`Patch ${id}: ${title}`}
      topRow={
        <>
          <Badge status={status} />
          <MonoId id={id} />
          <AgoTime iso={data.timestamp} />
        </>
      }
      title={title}
      bodyExcerpt={excerpt ?? undefined}
      footer={
        author || repoName ? (
          <>
            {author && (
              <span className={styles.assignee}>
                <Avatar name={author} kind="human" size="sm" />
                <span className={styles.assigneeName}>{author}</span>
              </span>
            )}
            {repoName && <span data-pc-mono="true">{repoName}</span>}
          </>
        ) : undefined
      }
    />
  );
}
