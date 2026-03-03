import { type KeyboardEvent, type ReactNode, useRef } from "react";
import styles from "./SearchBox.module.css";

export interface SearchBoxProps {
  value: string;
  onChange: (value: string) => void;
  onSettingsClick?: () => void;
  onSubmit?: () => void;
  placeholder?: string;
  leftElement?: ReactNode;
}

export function SearchBox({
  value,
  onChange,
  onSettingsClick,
  onSubmit,
  placeholder = "Search issues...",
  leftElement,
}: SearchBoxProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      onChange("");
      inputRef.current?.blur();
    } else if (e.key === "Enter") {
      e.preventDefault();
      onSubmit?.();
    }
  }

  return (
    <div className={styles.container} onClick={() => inputRef.current?.focus()}>
      {leftElement}
      <div className={styles.searchIcon} aria-hidden>
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
          <circle cx="7" cy="7" r="5.5" stroke="currentColor" strokeWidth="1.5" />
          <line x1="11" y1="11" x2="14.5" y2="14.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </div>
      <input
        ref={inputRef}
        className={styles.input}
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
      />
      {onSettingsClick && (
        <button
          type="button"
          className={styles.iconButton}
          onClick={(e) => {
            e.stopPropagation();
            onSettingsClick();
          }}
          aria-label="Search settings"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <line x1="2" y1="4" x2="14" y2="4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            <line x1="2" y1="8" x2="14" y2="8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            <line x1="2" y1="12" x2="14" y2="12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            <circle cx="5" cy="4" r="1.5" fill="var(--color-bg-tertiary)" stroke="currentColor" strokeWidth="1.5" />
            <circle cx="10" cy="8" r="1.5" fill="var(--color-bg-tertiary)" stroke="currentColor" strokeWidth="1.5" />
            <circle cx="7" cy="12" r="1.5" fill="var(--color-bg-tertiary)" stroke="currentColor" strokeWidth="1.5" />
          </svg>
        </button>
      )}
    </div>
  );
}
