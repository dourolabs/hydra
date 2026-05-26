import type { Principal } from "@hydra/api";

/**
 * Phase 4b of the actor-system overhaul moved attribution fields like
 * `Issue.assignee` from bare strings (`"alice"`) to typed
 * [`Principal`] objects (`{kind, name}` etc.). UI code that used to
 * read `assignee` directly now goes through these helpers. (Phase 5a
 * dropped the temporary `ActorPrincipal` rename — this type is now
 * exported as `Principal`.)
 */

/** Render a [`Principal`] as its canonical path form. */
export function formatPrincipalPath(principal: Principal): string {
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
 * Return the user-facing display name for a [`Principal`] — the
 * username component without the `users/` / `agents/` / `external/<sys>/`
 * prefix. Used wherever the UI previously rendered the bare string
 * `Issue.assignee`.
 */
export function principalDisplayName(principal: Principal): string {
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
  principal: Principal,
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
