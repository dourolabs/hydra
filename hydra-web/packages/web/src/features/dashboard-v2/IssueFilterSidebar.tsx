import { useCallback } from "react";
import { useKeyboardClick } from "@hydra/ui";
import styles from "./IssueFilterSidebar.module.css";

interface FilterItemProps {
  isActive: boolean;
  onClick: () => void;
  className: string;
  children: React.ReactNode;
}

function FilterItem({ isActive, onClick, className, children }: FilterItemProps) {
  const keyboardClickProps = useKeyboardClick(onClick);
  return (
    <li
      className={`${className} ${isActive ? styles.active : ""}`}
      onClick={onClick}
      {...keyboardClickProps}
    >
      {children}
    </li>
  );
}

interface IssueFilterSidebarProps {
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
  collapsed: boolean;
  drawerOpen: boolean;
  onDrawerClose: () => void;
  yourIssuesCount: number;
  assignedCount: number;
}

export function IssueFilterSidebar({
  activeFilter,
  onFilterChange,
  collapsed,
  drawerOpen,
  onDrawerClose,
  yourIssuesCount,
  assignedCount,
}: IssueFilterSidebarProps) {
  /** On mobile, selecting an issue should also close the drawer. */
  const handleFilterChange = useCallback(
    (rootId: string | null) => {
      onFilterChange(rootId);
      onDrawerClose();
    },
    [onFilterChange, onDrawerClose],
  );

  const handleYourIssuesClick = useCallback(
    () => handleFilterChange("your-issues"),
    [handleFilterChange],
  );
  const handleAssignedClick = useCallback(
    () => handleFilterChange("assigned"),
    [handleFilterChange],
  );

  const renderIssueList = (hideWhenCollapsed: boolean) => (
    <ul className={`${styles.list} ${hideWhenCollapsed && collapsed ? styles.listCollapsed : ""}`}>
      <FilterItem
        isActive={activeFilter === "your-issues"}
        onClick={handleYourIssuesClick}
        className={styles.item}
      >
        <span className={styles.itemLabel}>Your Issues</span>
        {yourIssuesCount > 0 && <span className={styles.inboxCount}>{yourIssuesCount}</span>}
      </FilterItem>
      <FilterItem
        isActive={activeFilter === "assigned"}
        onClick={handleAssignedClick}
        className={styles.item}
      >
        <span className={styles.itemLabel}>Assigned to You</span>
        {assignedCount > 0 && <span className={styles.inboxCount}>{assignedCount}</span>}
      </FilterItem>
    </ul>
  );

  return (
    <>
      {/* Desktop sidebar — hidden on mobile via CSS */}
      <div className={`${styles.sidebar} ${collapsed ? styles.collapsed : ""}`}>
        {!collapsed && (
          <div className={styles.header}>
            <span className={styles.title}>Issues</span>
          </div>
        )}
        {renderIssueList(true)}
      </div>

      {/* Mobile slide-out drawer (hamburger button lives in HeterogeneousItemList toolbar) */}
      {drawerOpen && <div className={styles.backdrop} onClick={onDrawerClose} />}
      <div className={`${styles.drawer} ${drawerOpen ? styles.drawerOpen : ""}`}>
        <div className={styles.drawerHeader}>
          <span className={styles.title}>Issues</span>
        </div>
        {renderIssueList(false)}
      </div>
    </>
  );
}
