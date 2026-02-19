import type { ActorIdentity, WhoAmIResponse } from "@metis/api";
import { apiFetch } from "./client";

export type { ActorIdentity, WhoAmIResponse };

/** Extract a display name from any actor identity. */
export function actorDisplayName(actor: ActorIdentity): string {
  return actor.type === "user" ? actor.username : actor.task_id;
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
