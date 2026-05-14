import styles from "./Sidebar.module.css";

export function ChatIcon() {
  return (
    <svg
      className={styles.sectionIcon}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        d="M18 10c0 3.866-3.582 7-8 7a8.841 8.841 0 01-4.083-.98L2 17l1.338-3.123C2.493 12.767 2 11.434 2 10c0-3.866 3.582-7 8-7s8 3.134 8 7zM7 9H5v2h2V9zm8 0h-2v2h2V9zm-5 0H8v2h2V9z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function IssuesIcon() {
  return (
    <svg
      className={styles.sectionIcon}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        d="M5 4a2 2 0 012-2h6a2 2 0 012 2v14l-5-2.5L5 18V4zm4.553 2.276a.75.75 0 00-1.06 1.06l1.25 1.25a.75.75 0 001.06 0l2.5-2.5a.75.75 0 10-1.06-1.06l-1.97 1.97-.72-.72z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function DocumentsIcon() {
  return (
    <svg
      className={styles.sectionIcon}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
    </svg>
  );
}

export function PatchesIcon() {
  return (
    <svg
      className={styles.sectionIcon}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        d="M5 3.75a1.75 1.75 0 100 3.5 1.75 1.75 0 000-3.5zM2 5.5a3 3 0 113.75 2.905v3.19a3.001 3.001 0 11-1.5 0v-3.19A3.001 3.001 0 012 5.5zm12.25-1.75a1.75 1.75 0 100 3.5 1.75 1.75 0 000-3.5zM11.25 5.5a3 3 0 116 0c0 1.298-.824 2.404-1.978 2.825-.227 1.652-.86 2.92-1.85 3.811-.93.836-2.07 1.27-3.197 1.55v.214a3.001 3.001 0 11-1.5 0V8.405a3.001 3.001 0 011.5 0v3.972c.875-.245 1.668-.583 2.293-1.146.673-.605 1.183-1.518 1.378-2.91A3.001 3.001 0 0111.25 5.5zM5 13.25a1.75 1.75 0 100 3.5 1.75 1.75 0 000-3.5zm5 0a1.75 1.75 0 100 3.5 1.75 1.75 0 000-3.5z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function AgentsIcon() {
  return (
    <svg
      className={styles.sectionIcon}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        d="M6 4.5a1 1 0 011-1h6a1 1 0 011 1V5h1.5A2.5 2.5 0 0118 7.5v6a2.5 2.5 0 01-2.5 2.5h-11A2.5 2.5 0 012 13.5v-6A2.5 2.5 0 014.5 5H6v-.5zm1.75 5a1.25 1.25 0 100 2.5 1.25 1.25 0 000-2.5zm4.5 0a1.25 1.25 0 100 2.5 1.25 1.25 0 000-2.5z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function ContextIcon() {
  return (
    <svg
      className={styles.sectionIcon}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M9.638 1.633a.75.75 0 01.724 0l7.25 4a.75.75 0 010 1.314l-7.25 4a.75.75 0 01-.724 0l-7.25-4a.75.75 0 010-1.314l7.25-4z" />
      <path d="M2.439 10.939a.75.75 0 011.02-.282L10 14.358l6.541-3.701a.75.75 0 11.738 1.305l-6.91 3.91a.75.75 0 01-.738 0l-6.91-3.91a.75.75 0 01-.282-1.023z" />
      <path d="M2.439 14.439a.75.75 0 011.02-.282L10 17.858l6.541-3.701a.75.75 0 11.738 1.305l-6.91 3.91a.75.75 0 01-.738 0l-6.91-3.91a.75.75 0 01-.282-1.023z" />
    </svg>
  );
}
