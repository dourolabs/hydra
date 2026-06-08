import type { Principal } from "@hydra/api";

export type AssignKind = "none" | "user" | "agent" | "external";

export function principalKind(p: Principal | null): AssignKind {
  if (!p) return "none";
  if ("Agent" in p) return "agent";
  if ("User" in p) return "user";
  return "external";
}

export function principalToPath(p: Principal): string {
  if ("Agent" in p) return `agents/${p.Agent.name}`;
  if ("User" in p) return `users/${p.User.name}`;
  return `external/${p.External.system}/${p.External.username}`;
}

export function pathToPrincipal(path: string): Principal | null {
  if (!path) return null;
  if (path.startsWith("agents/")) return { Agent: { name: path.slice(7) } };
  if (path.startsWith("users/")) return { User: { name: path.slice(6) } };
  if (path.startsWith("external/")) {
    const rest = path.slice("external/".length);
    const slash = rest.indexOf("/");
    if (slash < 0) return null;
    return {
      External: { system: rest.slice(0, slash), username: rest.slice(slash + 1) },
    };
  }
  return null;
}
