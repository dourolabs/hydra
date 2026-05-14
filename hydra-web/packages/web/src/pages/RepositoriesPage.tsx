import { RepositoriesSection } from "../features/repositories/RepositoriesSection";
import styles from "./RepositoriesPage.module.css";

export function RepositoriesPage() {
  return (
    <div className={styles.page}>
      <RepositoriesSection />
    </div>
  );
}
