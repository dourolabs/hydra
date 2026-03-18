import { Navigate, Outlet } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { useAuth } from "../features/auth/useAuth";
import { useSSE } from "../hooks/useSSE";
import { IconSidebar } from "./IconSidebar";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  const { user, loading } = useAuth();
  const sseState = useSSE();

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
