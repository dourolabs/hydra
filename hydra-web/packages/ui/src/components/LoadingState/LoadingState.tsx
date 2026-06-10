import { Spinner } from "../Spinner";
import styles from "./LoadingState.module.css";

export interface LoadingStateProps {
  message?: string;
  size?: "sm" | "md" | "lg";
}

export function LoadingState({ message, size = "md" }: LoadingStateProps) {
  return (
    <div className={styles.container}>
      <Spinner size={size} />
      {message && <p className={styles.message}>{message}</p>}
    </div>
  );
}
