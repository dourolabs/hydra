import { NavLink } from "react-router-dom";
import styles from "./TabBar.module.css";

const TABS = [
  { to: "/", label: "Dashboard", end: true },
  { to: "/issues", label: "Issues", end: false },
  { to: "/documents", label: "Documents", end: false },
  { to: "/patches", label: "Patches", end: false },
  { to: "/settings", label: "Settings", end: false },
] as const;

export function TabBar() {
  return (
    <nav className={styles.tabBar}>
      {TABS.map((tab) => (
        <NavLink
          key={tab.to}
          to={tab.to}
          end={tab.end}
          className={({ isActive }) =>
            `${styles.tab}${isActive ? ` ${styles.active}` : ""}`
          }
        >
          {tab.label}
        </NavLink>
      ))}
    </nav>
  );
}
