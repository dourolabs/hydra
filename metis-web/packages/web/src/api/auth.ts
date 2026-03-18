import type { ActorIdentity, WhoAmIResponse } from "@hydra/api";
import { apiFetch } from "./client";

/** Extract a display name from any actor identity. */
export function actorDisplayName(actor: ActorIdentity): string {
  if (actor.type === "user") return actor.username;
  if (actor.type === "session") return actor.session_id;
  if (actor.type === "service") return actor.service_name;
  return actor.issue_id;
}

export function login(token: string): Promise<WhoAmIResponse> {
  return apiFetch<WhoAmIResponse>("/auth/login", {
    method: "POST",
    body: JSON.stringify({ token }),
  });
}

export function logout(): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>("/auth/logout", {
    method: "POST",
  });
}

export function fetchMe(): Promise<WhoAmIResponse> {
  return apiFetch<WhoAmIResponse>("/auth/me");
}
