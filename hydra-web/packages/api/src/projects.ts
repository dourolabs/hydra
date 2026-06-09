/**
 * Project route request/response shapes.
 *
 * These mirror the hand-written request/response types in
 * `hydra-server/src/routes/projects.rs`. They are not ts-rs-generated
 * because the server side declares them outside `hydra-common` (only
 * the domain `Project` / `StatusDefinition` types are exported via ts-rs).
 */

import type { Project } from "./generated/Project";
import type { ProjectId } from "./generated/ProjectId";
import type { StatusDefinition } from "./generated/StatusDefinition";

/**
 * Seeded default project id. Must stay byte-identical to
 * `ProjectId::default_project()` in `hydra-common/src/ids.rs` and the id
 * inserted by `hydra-server/.../20260607000000_seed_default_project.sql`.
 */
export const DEFAULT_PROJECT_ID: ProjectId = "j-defaul";

export interface UpsertProjectRequest {
  project: Project;
}

export interface UpsertProjectResponse {
  project_id: ProjectId;
  version: number;
}

export interface ProjectRecord {
  project_id: ProjectId;
  version: number;
  project: Project;
}

export interface ListProjectsResponse {
  projects: ProjectRecord[];
}

export interface ProjectStatusesResponse {
  statuses: StatusDefinition[];
}
