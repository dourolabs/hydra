import type { ActorId, ActorRef } from "@hydra/api";

/** Extract a human-readable display name from an ActorId. */
export function actorIdDisplayName(id: ActorId): string {
  // Pre-migration `Legacy` rows round-trip as a bare string on the wire;
  // type-guard before any `in` test because `"x" in someString` throws.
  if (typeof id === "string") return id;
  if ("User" in id) return id.User.name;
  if ("Agent" in id) return id.Agent.name;
  if ("Adhoc" in id) return id.Adhoc.session_id;
  if ("External" in id)
    return `external/${id.External.system}/${id.External.username}`;
  if ("Username" in id) return id.Username;
  if ("Session" in id) return id.Session;
  if ("Issue" in id) return id.Issue;
  return id.Service;
}

/** Extract a human-readable display name from an ActorRef. */
export function actorDisplayName(actor: ActorRef): string {
  if ("Authenticated" in actor) {
    const id = actor.Authenticated.actor_id;
    return actorIdDisplayName(id);
  }
  if ("System" in actor) {
    const { worker_name, on_behalf_of } = actor.System;
    if (on_behalf_of) {
      return `${worker_name} (on behalf of ${actorIdDisplayName(on_behalf_of)})`;
    }
    return worker_name;
  }
  if ("Automation" in actor) {
    const { automation_name, triggered_by } = actor.Automation;
    if (triggered_by) {
      return `${automation_name} (triggered by ${actorDisplayName(triggered_by)})`;
    }
    return automation_name;
  }
  return "unknown";
}

/** Determine the short name used for the Avatar component. */
export function actorAvatarName(actor: ActorRef): string {
  if ("Authenticated" in actor) {
    const id = actor.Authenticated.actor_id;
    return actorIdDisplayName(id);
  }
  if ("System" in actor) return actor.System.worker_name;
  if ("Automation" in actor) return actor.Automation.automation_name;
  return "?";
}
