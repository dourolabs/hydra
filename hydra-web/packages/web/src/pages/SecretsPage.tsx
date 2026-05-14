import { SecretsSection } from "../features/secrets/SecretsSection";
import styles from "./SecretsPage.module.css";

export function SecretsPage() {
  return (
    <div className={styles.page}>
      <SecretsSection />
    </div>
  );
}
