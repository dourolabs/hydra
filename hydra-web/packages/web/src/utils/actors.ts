import type { ActorRef } from "@hydra/api";

/** Extract a human-readable display name from an ActorRef. */
export function actorDisplayName(actor: ActorRef): string {
  if ("Authenticated" in actor) {
    const id = actor.Authenticated.actor_id;
    if ("Username" in id) return id.Username;
    if ("Session" in id) return id.Session;
    if ("Service" in id) return id.Service;
    return id.Issue;
  }
  if ("System" in actor) {
    const { worker_name, on_behalf_of } = actor.System;
    if (on_behalf_of) {
      const name =
        "Username" in on_behalf_of
          ? on_behalf_of.Username
          : "Session" in on_behalf_of
            ? on_behalf_of.Session
            : "Service" in on_behalf_of
              ? on_behalf_of.Service
              : on_behalf_of.Issue;
      return `${worker_name} (on behalf of ${name})`;
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
    if ("Username" in id) return id.Username;
    if ("Session" in id) return id.Session;
    if ("Service" in id) return id.Service;
    return id.Issue;
  }
  if ("System" in actor) return actor.System.worker_name;
  if ("Automation" in actor) return actor.Automation.automation_name;
  return "?";
}
