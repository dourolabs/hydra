import styles from "./Avatar.module.css";

export interface AvatarProps {
  name: string;
  size?: "sm" | "md" | "lg";
  className?: string;
}

function getInitials(name: string): string {
  return name
    .split(/[\s_-]+/)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? "")
    .join("");
}

function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = str.charCodeAt(i) + ((hash << 5) - hash);
  }
  return Math.abs(hash);
}

const hues = [140, 200, 260, 320, 30, 60, 170, 290];

export function Avatar({ name, size = "md", className }: AvatarProps) {
  const initials = getInitials(name);
  const hue = hues[hashCode(name) % hues.length];
  const cls = [styles.avatar, styles[size], className].filter(Boolean).join(" ");

  return (
    <span
      className={cls}
      style={{ backgroundColor: `hsl(${hue}, 40%, 25%)`, color: `hsl(${hue}, 60%, 75%)` }}
      title={name}
      aria-label={name}
    >
      {initials}
    </span>
  );
}
