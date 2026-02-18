import { Avatar, Button } from "@metis/ui";
import { useAuth } from "../features/auth/AuthContext";
import styles from "./NavBar.module.css";

export function NavBar() {
  const { user, logout } = useAuth();

  return (
    <header className={styles.navbar}>
      <span className={styles.logo}>metis</span>
      {user && (
        <div className={styles.right}>
          <Avatar name={user.display_name ?? user.user_id} size="sm" />
          <span className={styles.username}>{user.display_name ?? user.user_id}</span>
          <Button variant="ghost" size="sm" onClick={logout}>
            Logout
          </Button>
        </div>
      )}
    </header>
  );
}
