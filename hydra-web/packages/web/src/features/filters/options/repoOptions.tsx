import { useMemo } from "react";
import { Icons } from "@hydra/ui";
import { useRepositories } from "../../../hooks/useRepositories";
import type { FilterOption } from "../types";
import styles from "./repoOptions.module.css";

/**
 * Returns repositories from the GET /v1/repositories endpoint as filter options.
 * `value` is the canonical `org/repo` name; chip and row both render the same
 * monospaced name with a repo icon.
 */
export function useRepoOptions(): FilterOption[] {
  const { data: repos } = useRepositories();

  return useMemo(() => {
    const out: FilterOption[] = [];
    for (const repo of repos ?? []) {
      out.push({
        value: repo.name,
        label: repo.name,
        chip: (
          <span className={styles.repoChip}>
            <Icons.IconRepo size={10} />
            <span>{repo.name}</span>
          </span>
        ),
        render: (
          <span className={styles.repoRow}>
            <Icons.IconRepo size={12} />
            <span>{repo.name}</span>
          </span>
        ),
      });
    }
    return out;
  }, [repos]);
}
