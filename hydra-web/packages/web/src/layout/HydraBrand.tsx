import { HydraLogo } from "../components/icons/HydraLogo";
import styles from "./HydraBrand.module.css";

export function HydraBrand() {
  return (
    <span className={styles.brand} aria-label="Hydra" data-testid="hydra-brand">
      <HydraLogo className={styles.glyph} />
      <span className={styles.wordmark}>Hydra</span>
    </span>
  );
}
