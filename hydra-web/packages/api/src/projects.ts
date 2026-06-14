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
import type { ProjectKey } from "./generated/ProjectKey";
import type { ProjectRef } from "./generated/ProjectRef";
import type { SessionSettings } from "./generated/SessionSettings";
import type { StatusDefinition } from "./generated/StatusDefinition";

export type { ProjectRef };

/**
 * Seeded default project id. Must stay byte-identical to
 * `ProjectId::default_project()` in `hydra-common/src/ids.rs` and the id
 * inserted by `hydra-server/.../20260607000000_seed_default_project.sql`.
 */
export const DEFAULT_PROJECT_ID: ProjectId = "j-defaul";

/**
 * Request body for `POST /v1/projects` and
 * `PUT /v1/projects/:project_ref`. Post-cutover this carries only
 * project-level fields; status add / update / delete go through the
 * per-status routes.
 */
export interface UpsertProjectRequest {
  key: ProjectKey;
  name: string;
  prompt_path?: string | null;
  priority?: number;
  /**
   * Per-project overrides for the `SessionSettings` applied when spawning
   * sessions for issues in this project. Mirrors the field on
   * `hydra-server`'s `Project` (and the ts-rs-generated
   * `UpsertProjectRequest`). The wire payload omits the field entirely
   * when undefined so the empty-collapse invariant on the UI form
   * round-trips unchanged.
   */
  session_settings?: SessionSettings;
}

export interface UpsertProjectResponse {
  project_id: ProjectId;
  version: number;
}

/**
 * Response body for `POST /v1/projects/:project_ref/statuses` and
 * `PUT /v1/projects/:project_ref/statuses/:status_key`.
 */
export interface UpsertProjectStatusResponse {
  project_id: ProjectId;
  version: number;
  status: StatusDefinition;
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
