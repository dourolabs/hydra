import { useCallback } from "react";
import styles from "./CopyButton.module.css";

export interface CopyButtonProps {
  value: string;
  onCopied?: () => void;
}

export function CopyButton({ value, onCopied }: CopyButtonProps) {
  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();
      navigator.clipboard.writeText(value).then(
        () => {
          onCopied?.();
        },
        (err) => {
          console.error("Failed to copy to clipboard:", err);
        },
      );
    },
    [value, onCopied],
  );

  return (
    <button
      type="button"
      className={styles.copyButton}
      onClick={handleClick}
      aria-label="Copy to clipboard"
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 16 16"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
      >
        <rect
          x="5.5"
          y="5.5"
          width="9"
          height="9"
          rx="1.5"
          stroke="currentColor"
          strokeWidth="1.5"
        />
        <path
          d="M10.5 5.5V3a1.5 1.5 0 0 0-1.5-1.5H3A1.5 1.5 0 0 0 1.5 3v6A1.5 1.5 0 0 0 3 10.5h2.5"
          stroke="currentColor"
          strokeWidth="1.5"
        />
      </svg>
    </button>
  );
}
