import styles from "./ChatRelatedTab.module.css";

const SECTIONS = [
  "Issues with active sessions",
  "Needs my attention",
  "Top-level issues",
  "Documents",
  "Patches",
];

export function ChatRelatedTab() {
  return (
    <div className={styles.relatedTab}>
      {SECTIONS.map((title) => (
        <section key={title} className={styles.section}>
          <h3 className={styles.sectionTitle}>{title}</h3>
          <p className={styles.empty}>(empty)</p>
        </section>
      ))}
    </div>
  );
}
