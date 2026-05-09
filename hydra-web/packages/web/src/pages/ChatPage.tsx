import { useParams } from "react-router-dom";
import { Breadcrumbs } from "../layout/Breadcrumbs";
import styles from "./ChatPage.module.css";

export function ChatPage() {
  const { conversationId } = useParams<{ conversationId: string }>();

  return (
    <div className={styles.page}>
      <Breadcrumbs items={[{ label: "Chat", to: "/chat" }]} current={conversationId ?? ""} />
      <p className={styles.placeholder}>Conversation: {conversationId}</p>
    </div>
  );
}
