import { Avatar, Button } from "@metis/ui";
import { useAuth } from "../features/auth/AuthContext";
import { actorDisplayName } from "../api/auth";
import styles from "./NavBar.module.css";

export function NavBar() {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;

  return (
    <header className={styles.navbar}>
      <span className={styles.logo}>metis</span>
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
