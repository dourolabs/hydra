import { Navigate } from "react-router-dom";
import { useAuth } from "../features/auth/AuthContext";
import { LoginForm } from "../features/auth/LoginForm";
import styles from "./LoginPage.module.css";

export function LoginPage() {
  const { user, loading } = useAuth();

  if (loading) return null;
  if (user) return <Navigate to="/" replace />;

  return (
    <div className={styles.page}>
      <div className={styles.card}>
        <h1 className={styles.title}>metis</h1>
        <p className={styles.subtitle}>Sign in to continue</p>
        <LoginForm />
      </div>
    </div>
  );
}
