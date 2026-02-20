import { Avatar, Button } from "@metis/ui";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import type { SSEConnectionState } from "../hooks/useSSE";
import styles from "./NavBar.module.css";

interface NavBarProps {
  connectionState: SSEConnectionState;
}

const CONNECTION_LABELS: Record<SSEConnectionState, string> = {
  connected: "Live",
  connecting: "Connecting",
  disconnected: "Offline",
};

export function NavBar({ connectionState }: NavBarProps) {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;

  return (
    <header className={styles.navbar}>
      <div className={styles.left}>
        <span className={styles.logo}>metis</span>
        <span
          className={`${styles.connectionStatus} ${styles[connectionState]}`}
          title={`SSE: ${CONNECTION_LABELS[connectionState]}`}
        >
          <span className={styles.dot} />
          <span className={styles.connectionLabel}>
            {CONNECTION_LABELS[connectionState]}
          </span>
        </span>
      </div>
      {user && displayName && (
        <div className={styles.right}>
          <Avatar name={displayName} size="sm" />
          <span className={styles.username}>{displayName}</span>
          <Button variant="ghost" size="sm" onClick={logout}>
            Logout
          </Button>
        </div>
      )}
    </header>
  );
}
