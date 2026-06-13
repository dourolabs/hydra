import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Icons } from "@hydra/ui";
import { useMediaQuery } from "../../hooks/useMediaQuery";
import styles from "./CollapsibleSearch.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

export interface CollapsibleSearchProps {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  ariaLabel: string;
  testId?: string;
}

export function CollapsibleSearch({
  value,
  onChange,
  placeholder,
  ariaLabel,
  testId,
}: CollapsibleSearchProps) {
  const isMobile = useMediaQuery(MOBILE_QUERY);
  // A non-empty filter on first paint is a strong signal the user wants the
  // input visible (e.g. landing on /?q=foo); after that, expand/collapse is
  // driven by explicit user actions.
  const [expanded, setExpanded] = useState(value !== "");
  const inputRef = useRef<HTMLInputElement>(null);
  const focusOnNextRenderRef = useRef(false);

  useEffect(() => {
    if (focusOnNextRenderRef.current) {
      inputRef.current?.focus();
      focusOnNextRenderRef.current = false;
    }
  }, [expanded]);

  function handleExpand() {
    focusOnNextRenderRef.current = true;
    setExpanded(true);
  }

  function handleCollapse() {
    setExpanded(false);
  }

  function handleClear() {
    onChange("");
    setExpanded(false);
  }

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      e.preventDefault();
      handleCollapse();
    }
  }

  // Desktop is always inline. On mobile we collapse to an icon button unless
  // the user has explicitly expanded or arrived with a non-empty value.
  const showInput = !isMobile || expanded;

  if (!showInput) {
    return (
      <button
        type="button"
        className={styles.iconButton}
        onClick={handleExpand}
        aria-label={ariaLabel}
        aria-expanded="false"
        data-testid={testId ? `${testId}-toggle` : undefined}
      >
        <Icons.IconSearch size={18} />
      </button>
    );
  }

  return (
    <div className={styles.box}>
      <span className={styles.icon} aria-hidden>
        <Icons.IconSearch size={14} />
      </span>
      <input
        ref={inputRef}
        type="text"
        className={styles.input}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        aria-label={ariaLabel}
        data-testid={testId}
      />
      {isMobile && (
        <button
          type="button"
          className={styles.clearButton}
          onClick={handleClear}
          aria-label="Clear search"
          data-testid={testId ? `${testId}-clear` : undefined}
        >
          <Icons.IconX size={14} />
        </button>
      )}
    </div>
  );
}
