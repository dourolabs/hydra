import type { SessionSummaryRecord } from "@hydra/api";
import { RunTime } from "../../../components/Runtime/Runtime";
import { useSessionDuration } from "../../dashboard/useSessionDuration";

export function CardRuntime({
  sessions,
}: {
  sessions: SessionSummaryRecord[] | undefined;
}) {
  const { durationText, status } = useSessionDuration(sessions);
  if (durationText === "—") return null;
  return <RunTime value={durationText} status={status} />;
}
