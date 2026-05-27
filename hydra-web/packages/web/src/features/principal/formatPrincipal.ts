import type { Principal } from "@hydra/api";

/**
 * Phase 4b of the actor-system overhaul moved attribution fields like
 * `Issue.assignee` from bare strings (`"alice"`) to typed
 * [`Principal`] objects (`{ User: { name } }` etc.). UI code that
 * used to read `assignee` directly now goes through these helpers.
 * (Phase 5a dropped the temporary `ActorPrincipal` rename — this
 * type is now exported as `Principal`.)
 */

/** Render a [`Principal`] as its canonical path form. */
export function formatPrincipalPath(principal: Principal): string {
  if ("User" in principal) return `users/${principal.User.name}`;
  if ("Agent" in principal) return `agents/${principal.Agent.name}`;
  return `external/${principal.External.system}/${principal.External.username}`;
}

/**
 * Return the user-facing display name for a [`Principal`] — the
 * username component without the `users/` / `agents/` / `external/<sys>/`
 * prefix. Used wherever the UI previously rendered the bare string
 * `Issue.assignee`.
 */
export function principalDisplayName(principal: Principal): string {
  if ("User" in principal) return principal.User.name;
  if ("Agent" in principal) return principal.Agent.name;
  return principal.External.username;
}

/**
 * Avatar `kind` hint derived from the principal's variant. The
 * `<Avatar />` component currently understands only `"human"` and
 * `"agent"`, so external principals fall through to `"human"`.
 */
export function principalAvatarKind(
  principal: Principal,
): "human" | "agent" {
  if ("Agent" in principal) return "agent";
  return "human";
}
