import { SecretsSection } from "../features/secrets/SecretsSection";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./SecretsPage.module.css";

export function SecretsPage() {
  useBreadcrumbs([], "Secrets");
  return (
    <div className={styles.page}>
      <SecretsSection />
    </div>
  );
}
