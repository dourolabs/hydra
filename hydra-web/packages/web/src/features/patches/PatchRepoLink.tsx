import type { PatchSummaryRecord } from "@hydra/api";
import { Icons } from "@hydra/ui";
import styles from "./PatchRepoLink.module.css";

type PatchRepoFields = Pick<
  PatchSummaryRecord["patch"],
  "github" | "service_repo_name"
>;

interface PatchRepoLinkProps {
  patch: PatchRepoFields;
}

export function PatchRepoLink({ patch }: PatchRepoLinkProps) {
  if (patch.github?.url) {
    const { url, owner, repo, number } = patch.github;
    return (
      <a
        href={url}
        target="_blank"
        rel="noopener noreferrer"
        className={styles.link}
        onClick={(e) => e.stopPropagation()}
      >
        <span className={styles.label}>
          {owner}/{repo}#{String(number)}
        </span>
        <Icons.IconExternalLink size={12} className={styles.icon} aria-hidden="true" />
      </a>
    );
  }
  return <>{patch.service_repo_name}</>;
}
