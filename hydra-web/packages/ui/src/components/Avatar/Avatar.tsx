import styles from "./Avatar.module.css";

export interface AvatarProps {
  name: string;
  kind?: "human" | "agent";
  size?: "sm" | "md" | "lg";
  className?: string;
  /** Overrides the hover tooltip and aria-label. Defaults to `name`. */
  title?: string;
}

function getInitials(name: string): string {
  const trimmed = (name || "?").trim();
  // Take first letter of first two words, fall back to first two chars.
  const parts = trimmed.split(/[\s_-]+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0]![0]! + parts[1]![0]!).toLowerCase();
  }
  return trimmed.slice(0, 2).toLowerCase();
}

export function Avatar({ name, kind = "human", size = "md", className, title }: AvatarProps) {
  const initials = getInitials(name);
  const cls = [styles.avatar, styles[size], className].filter(Boolean).join(" ");
  const label = title ?? name;

  return (
    <span className={cls} data-kind={kind} title={label} aria-label={label}>
      {initials}
    </span>
  );
}
