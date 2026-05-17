import styles from "./MobileTabBar.module.css";

export interface MobileTabBarItem {
  key: string;
  label: string;
}

interface MobileTabBarProps {
  tabs: MobileTabBarItem[];
  activeKey: string;
  onChange: (key: string) => void;
  testIdPrefix?: string;
  className?: string;
}

export function MobileTabBar({
  tabs,
  activeKey,
  onChange,
  testIdPrefix = "mobile-tab-",
  className,
}: MobileTabBarProps) {
  return (
    <div className={className ? `${styles.bar} ${className}` : styles.bar} role="tablist">
      {tabs.map((t) => {
        const isActive = activeKey === t.key;
        return (
          <button
            key={t.key}
            type="button"
            role="tab"
            aria-selected={isActive}
            className={`${styles.tab}${isActive ? ` ${styles.tabActive}` : ""}`}
            onClick={() => onChange(t.key)}
            data-testid={`${testIdPrefix}${t.key}`}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}
