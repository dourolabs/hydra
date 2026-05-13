import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { useNavigate } from "react-router-dom";
import { useGlobalSearch } from "./useGlobalSearch";
import {
  conversationToItem,
  documentToItem,
  issueToItem,
  patchToItem,
  sessionToItem,
  type SearchItem,
  type SearchItemKind,
} from "./searchItems";
import styles from "./GlobalSearchModal.module.css";

interface GlobalSearchModalProps {
  open: boolean;
  onClose: () => void;
}

interface SearchGroup {
  kind: SearchItemKind;
  heading: string;
  items: SearchItem[];
}

const GROUP_HEADINGS: Record<SearchItemKind, string> = {
  issue: "Issues",
  patch: "Patches",
  document: "Documents",
  conversation: "Chats",
  session: "Sessions",
};

const DEBOUNCE_MS = 200;

export function GlobalSearchModal({ open, onClose }: GlobalSearchModalProps) {
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);
  const [rawQuery, setRawQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);

  useEffect(() => {
    if (!open) {
      setRawQuery("");
      setDebouncedQuery("");
      setSelectedIndex(0);
      return;
    }
    previouslyFocused.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    // Focus the input after the modal renders.
    const t = window.setTimeout(() => {
      inputRef.current?.focus();
    }, 0);
    return () => {
      window.clearTimeout(t);
      const prev = previouslyFocused.current;
      previouslyFocused.current = null;
      if (prev && document.contains(prev)) {
        prev.focus();
      }
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const t = window.setTimeout(() => {
      setDebouncedQuery(rawQuery.trim());
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [rawQuery, open]);

  // Escape closes the modal regardless of focus.
  useEffect(() => {
    if (!open) return;
    const handler = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose]);

  const results = useGlobalSearch(debouncedQuery);

  const groups = useMemo<SearchGroup[]>(() => {
    const all: SearchGroup[] = [
      {
        kind: "issue",
        heading: GROUP_HEADINGS.issue,
        items: results.issues.map(issueToItem),
      },
      {
        kind: "patch",
        heading: GROUP_HEADINGS.patch,
        items: results.patches.map(patchToItem),
      },
      {
        kind: "document",
        heading: GROUP_HEADINGS.document,
        items: results.documents.map(documentToItem),
      },
      {
        kind: "conversation",
        heading: GROUP_HEADINGS.conversation,
        items: results.conversations.map(conversationToItem),
      },
      {
        kind: "session",
        heading: GROUP_HEADINGS.session,
        items: results.sessions.map(sessionToItem),
      },
    ];
    return all.filter((g) => g.items.length > 0);
  }, [results]);

  const flatItems = useMemo<SearchItem[]>(
    () => groups.flatMap((g) => g.items),
    [groups],
  );

  // Reset selection when the query or result set changes.
  useEffect(() => {
    setSelectedIndex(0);
  }, [debouncedQuery, flatItems.length]);

  const goTo = useCallback(
    (item: SearchItem) => {
      if (!item.href) return;
      onClose();
      navigate(item.href);
    },
    [navigate, onClose],
  );

  const handleInputKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        if (flatItems.length === 0) return;
        setSelectedIndex((prev) => (prev + 1) % flatItems.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        if (flatItems.length === 0) return;
        setSelectedIndex((prev) =>
          prev <= 0 ? flatItems.length - 1 : prev - 1,
        );
      } else if (event.key === "Enter") {
        event.preventDefault();
        const item = flatItems[selectedIndex];
        if (item) goTo(item);
      }
    },
    [flatItems, selectedIndex, goTo],
  );

  const handleBackdropClick = useCallback(
    (event: MouseEvent<HTMLDivElement>) => {
      if (event.target === event.currentTarget) {
        onClose();
      }
    },
    [onClose],
  );

  const handleInputChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      setRawQuery(event.target.value);
    },
    [],
  );

  if (!open) return null;

  const hasQuery = debouncedQuery.length > 0;
  const noResults =
    hasQuery && !results.isLoading && flatItems.length === 0;

  let flatCursor = 0;

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
        aria-label="Search"
        data-testid="global-search-modal"
      >
        <div className={styles.inputRow}>
          <svg
            className={styles.inputIcon}
            viewBox="0 0 20 20"
            fill="currentColor"
            aria-hidden="true"
          >
            <path
              fillRule="evenodd"
              d="M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z"
              clipRule="evenodd"
            />
          </svg>
          <input
            ref={inputRef}
            type="text"
            className={styles.input}
            placeholder="Search issues, patches, documents, chats, sessions…"
            value={rawQuery}
            onChange={handleInputChange}
            onKeyDown={handleInputKeyDown}
            data-testid="global-search-input"
            aria-label="Search query"
            aria-autocomplete="list"
            aria-controls="global-search-results"
            autoComplete="off"
            spellCheck={false}
          />
        </div>

        <div
          id="global-search-results"
          className={styles.results}
          data-testid="global-search-results"
          role="listbox"
        >
          {!hasQuery && (
            <div
              className={styles.hint}
              data-testid="global-search-empty-hint"
            >
              Type to search…
            </div>
          )}

          {noResults && (
            <div
              className={styles.hint}
              data-testid="global-search-no-results"
            >
              No results
            </div>
          )}

          {hasQuery &&
            groups.map((group) => (
              <div
                key={group.kind}
                className={styles.group}
                data-testid={`global-search-group-${group.kind}`}
              >
                <div className={styles.groupHeading}>{group.heading}</div>
                <ul className={styles.list}>
                  {group.items.map((item) => {
                    const index = flatCursor++;
                    const selected = index === selectedIndex;
                    return (
                      <li key={`${item.kind}-${item.id}`}>
                        <button
                          type="button"
                          role="option"
                          aria-selected={selected}
                          className={`${styles.row}${selected ? ` ${styles.rowSelected}` : ""}${item.href ? "" : ` ${styles.rowDisabled}`}`}
                          onClick={() => goTo(item)}
                          onMouseEnter={() => setSelectedIndex(index)}
                          disabled={!item.href}
                          data-testid={`global-search-row-${item.kind}-${item.id}`}
                          title={item.label}
                        >
                          <span className={styles.rowLabel}>{item.label}</span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
        </div>
      </div>
    </div>
  );
}
