import { useCallback, useEffect, useRef, useState } from "react";
import styles from "./CopyButton.module.css";

export interface CopyButtonProps {
  value: string;
  onCopied?: () => void;
}

function fallbackCopyText(text: string): boolean {
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  textarea.style.top = "-9999px";
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();
  let success = false;
  try {
    success = document.execCommand("copy");
  } catch {
    success = false;
  }
  document.body.removeChild(textarea);
  return success;
}

type CopyState = "idle" | "copied" | "failed";

export function CopyButton({ value, onCopied }: CopyButtonProps) {
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  const handleClick = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();

      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }

      let success = false;
      try {
        if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
          await navigator.clipboard.writeText(value);
          success = true;
        } else {
          success = fallbackCopyText(value);
        }
      } catch {
        success = fallbackCopyText(value);
      }

      if (success) {
        setCopyState("copied");
        onCopied?.();
      } else {
        setCopyState("failed");
        console.error("Failed to copy to clipboard");
      }

      timerRef.current = setTimeout(() => {
        setCopyState("idle");
        timerRef.current = null;
      }, 2000);
    },
    [value, onCopied],
  );

  return (
    <button
      type="button"
      className={`${styles.copyButton} ${copyState === "copied" ? styles.copied : ""} ${copyState === "failed" ? styles.failed : ""}`}
      onClick={handleClick}
      aria-label="Copy to clipboard"
    >
      {copyState === "copied" ? (
        <svg
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
          aria-hidden="true"
        >
          <path
            d="M3 8.5L6.5 12L13 4"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      ) : copyState === "failed" ? (
        <svg
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
          aria-hidden="true"
        >
          <path
            d="M4 4L12 12M12 4L4 12"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
        </svg>
      ) : (
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
      )}
    </button>
  );
}
