import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Icons } from "@hydra/ui";
import { useIsMobile } from "../../hooks/useIsMobile";
import type { Filter, FilterDefinitions } from "./types";
import { FilterChip } from "./FilterChip";
import { AddFilterMenu } from "./AddFilterMenu";
import { ValuePicker } from "./ValuePicker";
import styles from "./FilterBar.module.css";

interface FilterBarProps<T> {
  filters: Filter[];
  setFilters: (next: Filter[]) => void;
  definitions: FilterDefinitions<T>;
  count: number;
  total: number;
  /**
   * Notified whenever the add-filter menu opens or closes. Pages that
   * lazy-load relation picker options use this to start prefetching the
   * moment the menu opens so the picker isn't empty when clicked.
   */
  onMenuOpenChange?: (open: boolean) => void;
}

const POP_GAP = 4;
const POP_MAX_HEIGHT = 420;

let uidCounter = 0;
function makeUid(): string {
  uidCounter += 1;
  const rand = Math.random().toString(36).slice(2, 8);
  return `f${uidCounter}_${rand}`;
}

function computeAnchor(
  rect: DOMRect,
  popMaxHeight = POP_MAX_HEIGHT,
): CSSProperties {
  const viewportH = window.innerHeight;
  const spaceBelow = viewportH - rect.bottom;
  const spaceAbove = rect.top;
  const openUp =
    spaceBelow < popMaxHeight + POP_GAP && spaceAbove > spaceBelow;
  const style: CSSProperties = { left: rect.left };
  if (openUp) {
    style.bottom = viewportH - rect.top + POP_GAP;
  } else {
    style.top = rect.bottom + POP_GAP;
  }
  return style;
}

export function FilterBar<T>({
  filters,
  setFilters,
  definitions,
  count,
  total,
  onMenuOpenChange,
}: FilterBarProps<T>) {
  const addButtonRef = useRef<HTMLButtonElement | null>(null);
  const chipRefs = useRef<Map<string, HTMLDivElement | null>>(new Map());

  const [menuOpen, setMenuOpen] = useState(false);
  const [menuAnchor, setMenuAnchor] = useState<CSSProperties | null>(null);

  const [pickerOpenUid, setPickerOpenUid] = useState<string | null>(null);
  const [pickerAnchor, setPickerAnchor] = useState<CSSProperties | null>(null);

  // After adding a filter from the menu, we want to immediately open its
  // value picker. Setting `pendingOpenUid` flags it; a layout effect resolves
  // the chip's bounding rect once it's mounted and switches the picker on.
  const [pendingOpenUid, setPendingOpenUid] = useState<string | null>(null);

  const openMenu = useCallback(() => {
    const trigger = addButtonRef.current;
    if (!trigger) return;
    setMenuAnchor(computeAnchor(trigger.getBoundingClientRect()));
    setMenuOpen(true);
    onMenuOpenChange?.(true);
  }, [onMenuOpenChange]);

  const closeMenu = useCallback(() => {
    setMenuOpen(false);
    setMenuAnchor(null);
    onMenuOpenChange?.(false);
  }, [onMenuOpenChange]);

  const openPicker = useCallback((uid: string) => {
    const el = chipRefs.current.get(uid);
    if (!el) return;
    setPickerAnchor(computeAnchor(el.getBoundingClientRect()));
    setPickerOpenUid(uid);
  }, []);

  const closePicker = useCallback(() => {
    setPickerOpenUid(null);
    setPickerAnchor(null);
  }, []);

  useLayoutEffect(() => {
    if (!pendingOpenUid) return;
    const el = chipRefs.current.get(pendingOpenUid);
    if (!el) return;
    setPickerAnchor(computeAnchor(el.getBoundingClientRect()));
    setPickerOpenUid(pendingOpenUid);
    setPendingOpenUid(null);
  }, [pendingOpenUid]);

  // Reposition popovers when the viewport changes so they stay glued to their
  // anchors during resize/scroll. Capture-phase scroll catches ancestor scroll.
  useEffect(() => {
    if (!menuOpen && !pickerOpenUid) return;
    const reposition = () => {
      if (menuOpen && addButtonRef.current) {
        setMenuAnchor(computeAnchor(addButtonRef.current.getBoundingClientRect()));
      }
      if (pickerOpenUid) {
        const el = chipRefs.current.get(pickerOpenUid);
        if (el) setPickerAnchor(computeAnchor(el.getBoundingClientRect()));
      }
    };
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [menuOpen, pickerOpenUid]);

  const handlePick = useCallback(
    (id: string) => {
      const uid = makeUid();
      const next: Filter = { _uid: uid, id, op: "in", values: [] };
      setFilters([...filters, next]);
      closeMenu();
      // Presence filters carry no value, so skip the value-picker open step;
      // the chip is already active just by being on the bar.
      if (definitions[id]?.kind !== "presence") {
        setPendingOpenUid(uid);
      }
    },
    [filters, setFilters, closeMenu, definitions],
  );

  const handleRemove = useCallback(
    (uid: string) => {
      if (pickerOpenUid === uid) closePicker();
      setFilters(filters.filter((f) => f._uid !== uid));
    },
    [filters, setFilters, pickerOpenUid, closePicker],
  );

  const handleClearAll = useCallback(() => {
    closePicker();
    setFilters([]);
  }, [setFilters, closePicker]);

  const handleChange = useCallback(
    (next: Filter) => {
      setFilters(filters.map((f) => (f._uid === next._uid ? next : f)));
    },
    [filters, setFilters],
  );

  const openFilter = pickerOpenUid
    ? filters.find((f) => f._uid === pickerOpenUid) ?? null
    : null;
  const openDef = openFilter ? definitions[openFilter.id] ?? null : null;

  const isMobile = useIsMobile();
  const hasFilters = filters.length > 0;
  const hasActiveFilters = filters.some((f) => f.values.length > 0);
  const showSummary = !isMobile || hasActiveFilters;
  const summary =
    hasFilters && count !== total ? (
      <>
        <span className={styles.summaryCount}>{count}</span> of {total}
      </>
    ) : (
      `${total} results`
    );

  return (
    <div className={styles.bar} role="toolbar" aria-label="Filters">
      <div className={styles.chips}>
        {filters.map((filter) => {
          const def = definitions[filter.id];
          if (!def) return null;
          const isPresence = def.kind === "presence";
          return (
            <FilterChip
              key={filter._uid}
              filter={filter}
              definition={def}
              open={pickerOpenUid === filter._uid}
              onOpen={isPresence ? undefined : () => openPicker(filter._uid)}
              onRemove={() => handleRemove(filter._uid)}
              chipRef={(el) => {
                if (el) chipRefs.current.set(filter._uid, el);
                else chipRefs.current.delete(filter._uid);
              }}
            />
          );
        })}

        <button
          ref={addButtonRef}
          type="button"
          className={`${styles.addButton}${menuOpen ? ` ${styles.addButtonActive}` : ""}`}
          onClick={() => (menuOpen ? closeMenu() : openMenu())}
          aria-expanded={menuOpen}
          aria-haspopup="menu"
          aria-label="Add filter"
          data-testid="filter-bar-add"
        >
          <Icons.IconPlus size={12} />
          <span className={styles.addButtonLabel}>Filter</span>
          {filters.length >= 2 && (
            <span
              className={styles.addButtonBadge}
              aria-hidden="true"
              data-testid="filter-bar-add-badge"
            >
              {filters.length}
            </span>
          )}
        </button>

        {hasFilters && (
          <button
            type="button"
            className={styles.clearButton}
            onClick={handleClearAll}
            data-testid="filter-bar-clear-all"
          >
            Clear all
          </button>
        )}
      </div>

      {showSummary && (
        <span className={styles.summary} data-testid="filter-bar-summary">
          {summary}
        </span>
      )}

      {menuOpen && menuAnchor && (
        <AddFilterMenu
          definitions={definitions}
          anchorStyle={menuAnchor}
          onPick={handlePick}
          onClose={closeMenu}
        />
      )}

      {openFilter && openDef && pickerAnchor && (
        <ValuePicker
          filter={openFilter}
          definition={openDef}
          anchorStyle={pickerAnchor}
          onChange={handleChange}
          onClose={closePicker}
        />
      )}
    </div>
  );
}
