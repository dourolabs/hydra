import { Avatar, Button } from "@metis/ui";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : "";

  return (
    <div className={styles.page}>
      <h2 className={styles.title}>Settings</h2>
      <div className={styles.section}>
        <h3 className={styles.sectionTitle}>Account</h3>
        <div className={styles.userInfo}>
          <Avatar name={displayName} size="lg" />
          <div className={styles.userDetails}>
            <span className={styles.username}>{displayName}</span>
            <Button variant="secondary" size="sm" onClick={logout}>
              Logout
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
