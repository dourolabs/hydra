import styles from "./ProjectChip.module.css";

interface ProjectChipProps {
  /** Project key — rendered verbatim in monospace; no case forcing. */
  projectKey: string;
  /** Optional project name rendered after the key chip. */
  name?: string | null;
  className?: string;
  "data-testid"?: string;
}

export function ProjectChip({
  projectKey,
  name,
  className,
  "data-testid": testId,
}: ProjectChipProps) {
  const cls = [styles.wrap, className].filter(Boolean).join(" ");
  return (
    <span className={cls} data-testid={testId}>
      <span className={styles.key}>{projectKey}</span>
      {name ? <span className={styles.name}>{name}</span> : null}
    </span>
  );
}
