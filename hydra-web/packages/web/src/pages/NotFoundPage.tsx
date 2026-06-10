import { useNavigate } from "react-router-dom";
import { Button } from "@hydra/ui";
import styles from "./NotFoundPage.module.css";

export function NotFoundPage() {
  const navigate = useNavigate();

  return (
    <div className={styles.page}>
      <div className={styles.card}>
        <span className={styles.eyebrow}>404</span>
        <h1 className={styles.title}>Page not found</h1>
        <p className={styles.body}>The URL you requested doesn&rsquo;t match a known route.</p>
        <Button variant="primary" onClick={() => navigate("/")}>
          Go to dashboard
        </Button>
      </div>
    </div>
  );
}
