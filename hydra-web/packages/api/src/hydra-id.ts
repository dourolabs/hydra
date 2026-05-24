/**
 * HydraId prefix discrimination helpers.
 *
 * Mirrors the Rust enum in `hydra-common/src/ids.rs`. When a new HydraId kind
 * is added on the Rust side, update `HYDRA_ID_PREFIXES` here too so every
 * prefix lives in one location instead of scattered call-site `startsWith`
 * checks.
 *
 * Contract: these helpers only classify by *prefix*. They do NOT validate the
 * suffix character set or length — the Rust side (`HydraId::validate_str`) is
 * authoritative. On the TS side we trust the server: `i-anything` classifies
 * as `issue` and the consumer must tolerate ids the backend would reject.
 */

export type HydraIdKind = "issue" | "patch" | "document" | "session" | "label" | "conversation";

export const HYDRA_ID_PREFIXES: Record<HydraIdKind, string> = {
  issue: "i-",
  patch: "p-",
  document: "d-",
  session: "s-",
  label: "l-",
  conversation: "c-",
};

const HYDRA_ID_PREFIX_ORDER: ReadonlyArray<readonly [HydraIdKind, string]> = (
  Object.entries(HYDRA_ID_PREFIXES) as Array<[HydraIdKind, string]>
).sort((a, b) => b[1].length - a[1].length);

export function hydraIdKind(id: string): HydraIdKind | null {
  for (const [kind, prefix] of HYDRA_ID_PREFIX_ORDER) {
    if (id.startsWith(prefix)) return kind;
  }
  return null;
}

export function isIssueId(id: string): boolean {
  return hydraIdKind(id) === "issue";
}

export function isPatchId(id: string): boolean {
  return hydraIdKind(id) === "patch";
}

export function isDocumentId(id: string): boolean {
  return hydraIdKind(id) === "document";
}

export function isSessionId(id: string): boolean {
  return hydraIdKind(id) === "session";
}

export function isLabelId(id: string): boolean {
  return hydraIdKind(id) === "label";
}

export function isConversationId(id: string): boolean {
  return hydraIdKind(id) === "conversation";
}
