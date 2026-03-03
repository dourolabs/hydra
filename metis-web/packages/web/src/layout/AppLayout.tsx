import { Navigate, Outlet } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { useAuth } from "../features/auth/useAuth";
import { useSSE } from "../hooks/useSSE";
import { IconSidebar } from "./IconSidebar";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  const { user, loading } = useAuth();
  const currentUsername =
    user?.actor.type === "user" ? user.actor.username : undefined;
  const sseState = useSSE(currentUsername);

  if (loading) {
    return (
      <div className={styles.loading}>
        <Spinner size="lg" />
      </div>
    );
  }

  if (!user) {
    return <Navigate to="/login" replace />;
  }

  return (
    <div className={styles.layout}>
      <IconSidebar connectionState={sseState} />
      <main className={styles.main}>
        <Outlet />
      </main>
    </div>
  );
}
