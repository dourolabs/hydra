import type { JobSettings } from "@metis/api";
import styles from "./IssueSettings.module.css";

interface IssueSettingsProps {
  jobSettings?: JobSettings | null;
}

const FIELDS: { key: keyof JobSettings; label: string }[] = [
  { key: "repo_name", label: "Repository" },
  { key: "remote_url", label: "Remote URL" },
  { key: "image", label: "Image" },
  { key: "model", label: "Model" },
  { key: "branch", label: "Branch" },
  { key: "max_retries", label: "Max Retries" },
  { key: "cpu_limit", label: "CPU Limit" },
  { key: "memory_limit", label: "Memory Limit" },
];

export function IssueSettings({ jobSettings }: IssueSettingsProps) {
  if (!jobSettings) {
    return <p className={styles.empty}>No settings configured.</p>;
  }

  const entries = FIELDS.filter(
    (f) => jobSettings[f.key] != null,
  ).map((f) => ({
    label: f.label,
    value: String(jobSettings[f.key]),
  }));

  const secrets = jobSettings.secrets?.filter(Boolean);
  if (secrets && secrets.length > 0) {
    entries.push({ label: "Secrets", value: secrets.join(", ") });
  }

  if (entries.length === 0) {
    return <p className={styles.empty}>No settings configured.</p>;
  }

  return (
    <dl className={styles.list}>
      {entries.map((entry) => (
        <div key={entry.label} className={styles.row}>
          <dt className={styles.label}>{entry.label}</dt>
          <dd className={styles.value}>{entry.value}</dd>
        </div>
      ))}
    </dl>
  );
}
