import { type ReactNode } from "react";
import styles from "./Tabs.module.css";

export interface Tab {
  id: string;
  label: ReactNode;
}

export interface TabsProps {
  tabs: Tab[];
  activeTab: string;
  onTabChange: (id: string) => void;
  className?: string;
}

export function Tabs({ tabs, activeTab, onTabChange, className }: TabsProps) {
  const cls = [styles.tabs, className].filter(Boolean).join(" ");

  return (
    <div className={cls} role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          className={[styles.tab, tab.id === activeTab && styles.active].filter(Boolean).join(" ")}
          role="tab"
          aria-selected={tab.id === activeTab}
          onClick={() => onTabChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
