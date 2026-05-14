import type { Conversation } from "@hydra/api";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
}

export function ChatHeader({ conversation }: ChatHeaderProps) {
  const title = conversation.title || "Untitled conversation";

  return (
    <div className={styles.header}>
      <h2 className={styles.title}>{title}</h2>
    </div>
  );
}
