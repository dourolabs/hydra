import type { ActorPrincipal } from "@hydra/api";

/**
 * Phase 4b of the actor-system overhaul moved attribution fields like
 * `Issue.assignee` from bare strings (`"alice"`) to typed
 * [`ActorPrincipal`] objects (`{kind, name}` etc.). UI code that used to
 * read `assignee` directly now goes through these helpers.
 */

/** Render an [`ActorPrincipal`] as its canonical path form. */
export function formatPrincipalPath(principal: ActorPrincipal): string {
  switch (principal.kind) {
    case "user":
      return `users/${principal.name}`;
    case "agent":
      return `agents/${principal.name}`;
    case "external":
      return `external/${principal.system}/${principal.username}`;
  }
}

/**
 * Return the user-facing display name for an [`ActorPrincipal`] — the
 * username component without the `users/` / `agents/` / `external/<sys>/`
 * prefix. Used wherever the UI previously rendered the bare string
 * `Issue.assignee`.
 */
export function principalDisplayName(principal: ActorPrincipal): string {
  switch (principal.kind) {
    case "user":
    case "agent":
      return principal.name;
    case "external":
      return principal.username;
  }
}

/**
 * Avatar `kind` hint derived from the principal's variant. The
 * `<Avatar />` component currently understands only `"human"` and
 * `"agent"`, so external principals fall through to `"human"`.
 */
export function principalAvatarKind(
  principal: ActorPrincipal,
): "human" | "agent" {
  switch (principal.kind) {
    case "user":
      return "human";
    case "agent":
      return "agent";
    case "external":
      return "human";
  }
}
