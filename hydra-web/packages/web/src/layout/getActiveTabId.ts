export type MobileBottomTabId = "issues" | "patches" | "sessions" | "chat" | "more";

// Derives the active mobile bottom-tab from the current URL pathname.
// Lives in its own module (not next to MobileBottomTabBar) so the React
// Fast Refresh rule for one-component-per-file is satisfied.
export function getActiveTabId(pathname: string): MobileBottomTabId {
  // Strip a trailing slash so `/patches/` matches `/patches` exactly. The
  // root path `/` is left alone since stripping would change its meaning.
  const path = pathname.length > 1 && pathname.endsWith("/") ? pathname.slice(0, -1) : pathname;
  if (path === "/" || path === "/issues" || path.startsWith("/issues/")) return "issues";
  if (path === "/patches" || path.startsWith("/patches/")) return "patches";
  if (path === "/sessions" || path.startsWith("/sessions/")) return "sessions";
  if (path === "/chat" || path.startsWith("/chat/")) return "chat";
  return "more";
}
