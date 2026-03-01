import crypto from "node:crypto";

const PREFIXES = {
  issue: "i-",
  job: "t-",
  patch: "p-",
  document: "d-",
  notification: "nf-",
} as const;

export type EntityType = keyof typeof PREFIXES;

export function generateId(type: EntityType): string {
  const suffix = crypto.randomBytes(5).toString("hex").slice(0, 9);
  return `${PREFIXES[type]}${suffix}`;
}
