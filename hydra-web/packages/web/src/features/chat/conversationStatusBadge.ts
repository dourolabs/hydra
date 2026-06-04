import type { BadgeStatus } from "@hydra/ui";
import type { ConversationStatus } from "@hydra/api";

// Maps each `ConversationStatus` to its display BadgeStatus tone. The
// `unknown` forward-compat fallback variant (an older client sees a future
// server status as `unknown`) has no dedicated `conv-*` tone, so it folds
// into the generic neutral `unknown` BadgeStatus.
export const CONVERSATION_STATUS_TONES: Record<ConversationStatus, BadgeStatus> = {
  active: "conv-active",
  idle: "conv-idle",
  closed: "conv-closed",
  unknown: "unknown",
};

export const CONVERSATION_STATUS_LABELS: Record<ConversationStatus, string> = {
  active: "Active",
  idle: "Idle",
  closed: "Closed",
  unknown: "Unknown",
};
