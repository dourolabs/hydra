import { RepositoriesSection } from "../features/repositories/RepositoriesSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./RepositoriesPage.module.css";

export function RepositoriesPage() {
  useBreadcrumbs([], "Repositories");
  return (
    <div className={styles.page}>
      <RepositoriesSection />
    </div>
  );
}
