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
  ProjectRef,
  UpsertProjectRequest,
  UpsertProjectResponse,
  UpsertProjectStatusResponse,
  ProjectRecord,
  ListProjectsResponse,
  ProjectStatusesResponse,
} from "./projects";

export type { BucketGranularity } from "./generated/BucketGranularity";
export type { IssueType } from "./generated/IssueType";
export type { PatchesThroughputQuery } from "./generated/PatchesThroughputQuery";
export type { PatchesOverTimeResponse } from "./generated/PatchesOverTimeResponse";
export type { PatchOverTimeBucket } from "./generated/PatchOverTimeBucket";
export type { PatchesTerminalMixResponse } from "./generated/PatchesTerminalMixResponse";
export type { PatchesTimeToMergeResponse } from "./generated/PatchesTimeToMergeResponse";
export type { TimeToMergeBin } from "./generated/TimeToMergeBin";
export type { PatchesInFlightOverTimeResponse } from "./generated/PatchesInFlightOverTimeResponse";
export type { PatchInFlightBucket } from "./generated/PatchInFlightBucket";
export type { IssuesThroughputQuery } from "./generated/IssuesThroughputQuery";
export type { IssuesCycleTimeResponse } from "./generated/IssuesCycleTimeResponse";
export type { IssuesTimeInStatusBreakdownResponse } from "./generated/IssuesTimeInStatusBreakdownResponse";
export type { TimeInStatusSegment } from "./generated/TimeInStatusSegment";
export type { IssuesPerStatusDistributionResponse } from "./generated/IssuesPerStatusDistributionResponse";
export type { PerStatusDistribution } from "./generated/PerStatusDistribution";
export type { IssuesOverTimeResponse } from "./generated/IssuesOverTimeResponse";
export type { IssueOverTimeBucket } from "./generated/IssueOverTimeBucket";
