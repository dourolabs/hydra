import type { ConversationStatus, ConversationSummary } from "@hydra/api";

export function statusBucket(status: ConversationStatus): number {
  if (status === "active") return 0;
  if (status === "idle") return 1;
  return 2;
}

export function compareConversationsByBucketThenUpdated(
  a: ConversationSummary,
  b: ConversationSummary,
): number {
  const bucketDiff = statusBucket(a.status) - statusBucket(b.status);
  if (bucketDiff !== 0) return bucketDiff;
  return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
}
