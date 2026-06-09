import type { Project, StatusDefinition } from "@hydra/api";
import type { Store } from "./store.js";

/** Look up a project's `StatusDefinition` for the given key via the
 *  mock-server store. Mirrors `AppState::resolve_status` server-side:
 *  the project lives in the store, and the status is one of its
 *  declared entries. Throws when the project is missing or the key
 *  isn't declared — the prod server raises a 4xx in those cases too.
 */
export function resolveStatusDef(
  store: Store,
  projectId: string,
  key: string,
): StatusDefinition {
  const entry = store.get<Project>("projects", projectId);
  if (!entry) {
    throw new Error(
      `mock-server: project '${projectId}' not found while resolving status '${key}'`,
    );
  }
  const def = entry.data.statuses.find((s) => s.key === key);
  if (!def) {
    throw new Error(
      `mock-server: status '${key}' not declared on project '${projectId}'`,
    );
  }
  return def;
}
