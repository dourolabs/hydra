export { HydraApiClient } from "./client";
export type { HydraApiClientOptions } from "./client";

export { ApiError } from "./errors";

export {
  HYDRA_ID_PREFIXES,
  hydraIdKind,
  isIssueId,
  isPatchId,
  isDocumentId,
  isSessionId,
  isLabelId,
  isConversationId,
} from "./hydra-id";
export type { HydraIdKind } from "./hydra-id";

export { HydraEventSource, buildEventsUrl } from "./sse";
export type {
  HydraEvent,
  HydraEventHandler,
  HydraEventErrorHandler,
  EventSubscriptionOptions,
} from "./sse";

export * from "./types";

export { DEFAULT_PROJECT_ID } from "./projects";
export type {
  UpsertProjectRequest,
  UpsertProjectResponse,
  ProjectRecord,
  ListProjectsResponse,
  ProjectStatusesResponse,
} from "./projects";
