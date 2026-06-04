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
  default_status_key: string;
}
