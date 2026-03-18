import { Link } from "react-router-dom";
import type { Session } from "@hydra/api";
import { Badge } from "@hydra/ui";
import { normalizeSessionStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import styles from "./SessionSettings.module.css";

interface SessionSettingsProps {
  task: Session;
}

function formatContext(context: Session["context"]): string {
  switch (context.type) {
    case "git_repository":
      return `${context.url}${context.rev ? ` @ ${context.rev}` : ""}`;
    case "service_repository":
      return `${context.name}${context.rev ? ` @ ${context.rev}` : ""}`;
    case "none":
      return "None";
    default:
      return "Unknown";
  }
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
  const entries: { label: string; value: React.ReactNode }[] = [];

  if (task.prompt) {
    entries.push({
      label: "Prompt",
      value: <pre className={styles.prompt}>{task.prompt}</pre>,
    });
  }

  entries.push({ label: "Creator", value: task.creator });

  entries.push({
    label: "Status",
    value: <Badge status={normalizeSessionStatus(task.status)} />,
  });

  entries.push({ label: "Context", value: formatContext(task.context) });

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

  if (task.model) {
    entries.push({ label: "Model", value: task.model });
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
        <div key={entry.label} className={styles.row}>
          <dt className={styles.label}>{entry.label}</dt>
          <dd className={styles.value}>{entry.value}</dd>
        </div>
      ))}
    </dl>
  );
}
