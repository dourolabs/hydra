import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useNavigate } from "react-router-dom";
import {
  GROUP_ORDER,
  type GlobalSearchRow,
  useGlobalSearch,
} from "./useGlobalSearch";
import styles from "./GlobalSearchModal.module.css";

export interface GlobalSearchModalProps {
  open: boolean;
  onClose: () => void;
}

export function GlobalSearchModal({ open, onClose }: GlobalSearchModalProps) {
  const navigate = useNavigate();
  const [rawQuery, setRawQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  const { debouncedQuery, groups, flatRows, isLoading } = useGlobalSearch(rawQuery);

  // Reset query/selection on close, capture focus on open.
  useEffect(() => {
    if (open) {
      previousFocusRef.current =
        (document.activeElement as HTMLElement | null) ?? null;
      setRawQuery("");
      setSelectedIndex(0);
      const handle = requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
      return () => cancelAnimationFrame(handle);
    }
    const prev = previousFocusRef.current;
    previousFocusRef.current = null;
    if (prev && typeof prev.focus === "function") {
      // Defer restoration so the close action's own focus handler doesn't fight us.
      requestAnimationFrame(() => prev.focus());
    }
    return undefined;
  }, [open]);

  // Clamp the selected index when the result list shrinks.
  useEffect(() => {
    if (flatRows.length === 0) {
      if (selectedIndex !== 0) setSelectedIndex(0);
      return;
    }
    if (selectedIndex >= flatRows.length) {
      setSelectedIndex(flatRows.length - 1);
    }
  }, [flatRows.length, selectedIndex]);

  const visibleGroups = useMemo(
    () =>
      GROUP_ORDER.map(
        (type) => groups.find((g) => g.type === type)!,
      ).filter((g) => g.rows.length > 0),
    [groups],
  );

  const activateRow = useCallback(
    (row: GlobalSearchRow) => {
      if (row.to) {
        navigate(row.to);
        onClose();
      }
    },
    [navigate, onClose],
  );

  const handleInputKeyDown = useCallback(
    (e: ReactKeyboardEvent<HTMLInputElement>) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        if (flatRows.length > 0) {
          setSelectedIndex((i) => (i + 1) % flatRows.length);
        }
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        if (flatRows.length > 0) {
          setSelectedIndex((i) => (i - 1 + flatRows.length) % flatRows.length);
        }
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const row = flatRows[selectedIndex];
        if (row) activateRow(row);
      }
    },
    [activateRow, flatRows, selectedIndex],
  );

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.target === e.currentTarget) onClose();
    },
    [onClose],
  );

  if (!open) return null;

  const trimmed = debouncedQuery;
  const hasQuery = trimmed.length > 0;
  const hasResults = flatRows.length > 0;

  let bodyContent: React.ReactNode;
  if (!hasQuery) {
    bodyContent = (
      <div className={styles.hint} data-testid="global-search-hint">
        Type to search…
      </div>
    );
  } else if (!hasResults && !isLoading) {
    bodyContent = (
      <div className={styles.empty} data-testid="global-search-empty">
        No results
      </div>
    );
  } else {
    let runningIndex = 0;
    bodyContent = (
      <div className={styles.groups}>
        {visibleGroups.map((group) => (
          <div
            key={group.type}
            className={styles.group}
            data-testid={`global-search-group-${group.type}`}
          >
            <div className={styles.groupHeader}>{group.label}</div>
            <ul className={styles.rows}>
              {group.rows.map((row) => {
                const rowIndex = runningIndex++;
                const isSelected = rowIndex === selectedIndex;
                const className = `${styles.row}${isSelected ? ` ${styles.rowSelected}` : ""}`;
                return (
                  <li key={`${row.type}-${row.id}`}>
                    {row.to ? (
                      <button
                        type="button"
                        className={className}
                        data-testid={`global-search-row-${row.type}-${row.id}`}
                        onMouseEnter={() => setSelectedIndex(rowIndex)}
                        onClick={() => activateRow(row)}
                      >
                        <span className={styles.rowLabel}>{row.label}</span>
                      </button>
                    ) : (
                      <div
                        className={`${className} ${styles.rowDisabled}`}
                        data-testid={`global-search-row-${row.type}-${row.id}`}
                      >
                        <span className={styles.rowLabel}>{row.label}</span>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
    );
  }

  return (
    <div
      className={styles.backdrop}
      onClick={handleBackdropClick}
      data-testid="global-search-backdrop"
    >
      <div
        className={styles.modal}
        role="dialog"
        aria-modal="true"
        aria-label="Global search"
        data-testid="global-search-modal"
      >
        <div className={styles.inputWrap}>
          <input
            ref={inputRef}
            className={styles.input}
            type="text"
            value={rawQuery}
            placeholder="Search issues, patches, documents, chats, sessions…"
            onChange={(e) => setRawQuery(e.target.value)}
            onKeyDown={handleInputKeyDown}
            data-testid="global-search-input"
            aria-label="Search"
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        <div className={styles.body}>{bodyContent}</div>
      </div>
    </div>
  );
}
