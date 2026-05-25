import type { ActorId, ActorRef } from "@hydra/api";

/** Extract a human-readable display name from an ActorId. */
export function actorIdDisplayName(id: ActorId): string {
  if ("Username" in id) return id.Username;
  if ("Session" in id) return id.Session;
  if ("Issue" in id) return id.Issue;
  if ("Service" in id) return id.Service;
  if ("User" in id) return id.User;
  if ("Agent" in id) return id.Agent;
  if ("Adhoc" in id) return id.Adhoc;
  if ("External" in id) return `external/${id.External.system}/${id.External.username}`;
  return id.Legacy;
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
