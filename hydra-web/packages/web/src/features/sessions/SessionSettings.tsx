import { Link } from "react-router-dom";
import type { MountSpec, Session } from "@hydra/api";
import { Badge } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import { normalizeSessionStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import styles from "./SessionSettings.module.css";

interface SessionSettingsProps {
  task: Session;
}

function formatMountSpec(mountSpec: MountSpec): string {
  // PR-F: `Session.context` is gone; render the first Bundle item's URL
  // (with optional BuildCache service_repo_name overlay) so the UI keeps
  // showing the same "what does this session check out" hint.
  let bundleLabel: string | null = null;
  let serviceRepo: string | null = null;
  for (const mount of mountSpec.mounts) {
    if (mount.type === "bundle" && bundleLabel === null) {
      if (mount.bundle.type === "git_repository") {
        bundleLabel = `${mount.bundle.url}${mount.bundle.rev ? ` @ ${mount.bundle.rev}` : ""}`;
      } else if (mount.bundle.type === "none") {
        bundleLabel = "None";
      } else {
        bundleLabel = "Unknown";
      }
    } else if (mount.type === "build_cache") {
      serviceRepo = mount.service_repo_name;
    }
  }
  if (serviceRepo) {
    return bundleLabel ? `${serviceRepo} (${bundleLabel})` : serviceRepo;
  }
  return bundleLabel ?? "None";
}

function promptOf(task: Session): string | null {
  if (task.mode.type === "headless") return task.mode.prompt;
  return task.agent_config.system_prompt ?? null;
}

function formatError(error: Session["error"]): string | null {
  if (!error) return null;
  if (error === "unknown") return "Unknown error";
  return error.job_engine_error.reason;
}

function formatEnvVars(envVars: Session["env_vars"]): string | null {
  if (!envVars) return null;
  const keys = Object.keys(envVars);
  if (keys.length === 0) return null;
  return keys.join(", ");
}

export function SessionSettings({ task }: SessionSettingsProps) {
  const entries: { label: string; value: React.ReactNode; stacked?: boolean }[] =
    [];

  const prompt = promptOf(task);
  if (prompt) {
    entries.push({
      label: "Prompt",
      value: (
        <div className={styles.promptContent}>
          <Markdown content={prompt} />
        </div>
      ),
      stacked: true,
    });
  }

  entries.push({ label: "Creator", value: task.creator });

  entries.push({
    label: "Status",
    value: <Badge status={normalizeSessionStatus(task.status)} />,
  });

  entries.push({ label: "Context", value: formatMountSpec(task.mount_spec) });

  if (task.spawned_from) {
    entries.push({
      label: "Issue",
      value: (
        <Link to={`/issues/${task.spawned_from}`} className={styles.link}>
          {task.spawned_from}
        </Link>
      ),
    });
  }

  if (task.image) {
    entries.push({ label: "Image", value: task.image });
  }

  if (task.agent_config.model) {
    entries.push({ label: "Model", value: task.agent_config.model });
  }

  if (task.cpu_limit) {
    entries.push({ label: "CPU Limit", value: task.cpu_limit });
  }

  if (task.memory_limit) {
    entries.push({ label: "Memory Limit", value: task.memory_limit });
  }

  const envDisplay = formatEnvVars(task.env_vars);
  if (envDisplay) {
    entries.push({ label: "Environment", value: envDisplay });
  }

  const secrets = task.secrets?.filter(Boolean);
  if (secrets && secrets.length > 0) {
    entries.push({ label: "Secrets", value: secrets.join(", ") });
  }

  const errorDisplay = formatError(task.error);
  if (errorDisplay) {
    entries.push({ label: "Error", value: errorDisplay });
  }

  if (task.last_message) {
    entries.push({ label: "Last Message", value: task.last_message });
  }

  if (task.creation_time) {
    entries.push({
      label: "Created",
      value: formatTimestamp(task.creation_time),
    });
  }

  if (task.start_time) {
    entries.push({
      label: "Started",
      value: formatTimestamp(task.start_time),
    });
  }

  if (task.end_time) {
    entries.push({ label: "Ended", value: formatTimestamp(task.end_time) });
  }

  if (task.deleted) {
    entries.push({ label: "Deleted", value: "Yes" });
  }

  if (entries.length === 0) {
    return <p className={styles.empty}>No settings available.</p>;
  }

  return (
    <dl className={styles.list}>
      {entries.map((entry) => (
        <div
          key={entry.label}
          className={entry.stacked ? styles.rowStacked : styles.row}
        >
          <dt className={styles.label}>{entry.label}</dt>
          <dd className={styles.value}>{entry.value}</dd>
        </div>
      ))}
    </dl>
  );
}
