import crypto from "node:crypto";

const PREFIXES = {
  issue: "i-",
  session: "t-",
  patch: "p-",
  document: "d-",
  conversation: "c-",
  trigger: "t-",
} as const;

export type EntityType = keyof typeof PREFIXES;

export function generateId(type: EntityType): string {
  const suffix = crypto.randomBytes(5).toString("hex").slice(0, 9);
  return `${PREFIXES[type]}${suffix}`;
}
