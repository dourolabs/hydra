import { HydraBrand } from "./HydraBrand";
import styles from "./AppLayout.module.css";

interface AppChromeProps {
  hidden: boolean;
  onHide: () => void;
  onShow: () => void;
}

function HamburgerIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <path
        fillRule="evenodd"
        d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function AppChrome({ hidden, onHide, onShow }: AppChromeProps) {
  const onToggle = hidden ? onShow : onHide;
  const toggleLabel = hidden ? "Show sidebar" : "Hide sidebar";
  return (
    <div
      className={`${styles.leftChrome}${hidden ? ` ${styles.leftChromeOnHeader}` : ""}`}
      data-testid="app-left-chrome"
    >
      <button
        type="button"
        className={styles.chromeToggle}
        onClick={onToggle}
        aria-label={toggleLabel}
        data-testid="app-chrome-toggle-sidebar"
      >
        <HamburgerIcon />
      </button>
      <HydraBrand />
    </div>
  );
}
