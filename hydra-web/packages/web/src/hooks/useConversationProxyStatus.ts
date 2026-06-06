import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type {
  ConversationId,
  ProxyTarget,
  SessionSummaryRecord,
} from "@hydra/api";
import { apiClient } from "../api/client";
import { buildProxyUrl } from "../api/proxyAuth";

export type ProxyStatus = "starting" | "ready" | "idle" | "unavailable";

export interface ProxyTargetWithStatus extends ProxyTarget {
  /**
   * Combined status: `idle` whenever the conversation has no active session
   * (overrides the HEAD probe), otherwise driven by a periodic HEAD against
   * the proxy URL — `starting` while a probe is in-flight, `ready` once a
   * probe resolves, `unavailable` if a probe rejects.
   */
  status: ProxyStatus;
}

export interface ConversationProxyStatus {
  /**
   * Per-target status, in `Session.proxy_targets` order. Empty when the
   * conversation has no active session or no advertised targets — the tab
   * surfaces accordingly.
   */
  targets: ProxyTargetWithStatus[];
  /**
   * The session id whose `proxy_targets` are being surfaced. `null` when the
   * conversation has no session yet (fresh conversation pre-message) or when
   * the sessions list is still loading. The session-id is needed by the
   * "copy direct-session URL" affordance.
   */
  sessionId: string | null;
  /**
   * `true` while at least one underlying query is in-flight; the tab can
   * surface a spinner.
   */
  isLoading: boolean;
}

const POLL_INTERVAL_MS = 15_000;

/**
 * Locate the active session for a conversation. Mirrors
 * `useChatTranscript`'s "creation-time ascending, take last" rule: a
 * resumption chain orders sessions newest-last, and the only one a proxy
 * call can land on is the most recent.
 */
function pickActiveSession(
  records: readonly SessionSummaryRecord[] | undefined,
): string | null {
  if (!records || records.length === 0) return null;
  const sorted = [...records].sort((a, b) => {
    const at = a.session.creation_time ?? "";
    const bt = b.session.creation_time ?? "";
    return at.localeCompare(bt);
  });
  return sorted[sorted.length - 1].session_id;
}

function useDocumentVisible(): boolean {
  const [visible, setVisible] = useState(() =>
    typeof document === "undefined" ? true : !document.hidden,
  );
  useEffect(() => {
    if (typeof document === "undefined") return;
    const handler = () => setVisible(!document.hidden);
    document.addEventListener("visibilitychange", handler);
    return () => document.removeEventListener("visibilitychange", handler);
  }, []);
  return visible;
}

/**
 * Periodically HEAD-probes every target so the row can render a current
 * `starting | ready | unavailable` status. The HEAD goes cross-origin to the
 * proxy subdomain in `no-cors` mode — the response is opaque (we cannot read
 * the status code), but the promise's resolution distinguishes "subdomain
 * reachable" from "DNS / network failure," which is the live/dead signal the
 * UI needs. Polling is suspended while the document is hidden so a backgrounded
 * tab does not keep pinging.
 */
function useTargetProbes(
  targets: readonly ProxyTarget[],
  targetLabel: string | null,
  paused: boolean,
): Map<number, ProxyStatus> {
  const [statuses, setStatuses] = useState<Map<number, ProxyStatus>>(new Map());
  const cancelledRef = useRef(false);

  useEffect(() => {
    cancelledRef.current = false;
    if (paused || !targetLabel || targets.length === 0) {
      setStatuses(new Map());
      return () => {
        cancelledRef.current = true;
      };
    }

    const probe = async () => {
      // Mark every target `starting` for the duration of this round, then
      // flip per-target as each probe lands. This matches the design's
      // in-flight → starting semantic and avoids a flash of stale status
      // when the convo flips active.
      setStatuses((prev) => {
        const next = new Map(prev);
        for (const t of targets) {
          if (!next.has(t.port)) next.set(t.port, "starting");
        }
        return next;
      });
      await Promise.all(
        targets.map(async (target) => {
          const url = buildProxyUrl({
            port: target.port,
            targetLabel,
            readyPath: target.ready_path ?? null,
            mainHost: window.location.host,
            protocol: window.location.protocol,
          });
          let status: ProxyStatus;
          try {
            await fetch(url, {
              method: "HEAD",
              mode: "no-cors",
              credentials: "include",
              cache: "no-store",
            });
            status = "ready";
          } catch {
            status = "unavailable";
          }
          if (cancelledRef.current) return;
          setStatuses((prev) => {
            const next = new Map(prev);
            next.set(target.port, status);
            return next;
          });
        }),
      );
    };

    void probe();
    const handle = window.setInterval(probe, POLL_INTERVAL_MS);
    return () => {
      cancelledRef.current = true;
      window.clearInterval(handle);
    };
    // Re-run when the target list or convo/session label changes; `paused`
    // re-runs on visibility flips.
  }, [targets, targetLabel, paused]);

  return statuses;
}

/**
 * Live status for the proxy targets advertised on a conversation's currently
 * active session.
 *
 * Combines three signals:
 *   1. Conversation status (`idle` overrides everything else with the
 *      "send a message to resume" copy).
 *   2. The active session's `proxy_targets` list (refetched on every chat-page
 *      mount, same cadence as the transcript).
 *   3. A periodic HEAD against each `<port>-<conv-id>.proxy.<host>{ready_path}`
 *      URL to distinguish "dev server is up" from "advertised but unreachable."
 */
export function useConversationProxyStatus(
  conversationId: string,
): ConversationProxyStatus {
  const conversationQuery = useQuery({
    queryKey: ["conversation", conversationId],
    queryFn: () => apiClient.getConversation(conversationId),
    enabled: !!conversationId,
  });

  const sessionsQuery = useQuery({
    queryKey: ["sessionsByConversation", conversationId],
    queryFn: () =>
      apiClient.listSessions({
        conversation_id: conversationId as unknown as ConversationId,
      }),
    enabled: !!conversationId,
    refetchOnMount: "always",
  });

  const activeSessionId = useMemo(
    () => pickActiveSession(sessionsQuery.data?.sessions),
    [sessionsQuery.data],
  );

  // Pull `proxy_targets` off the full Session record. The list view's
  // `SessionSummary` deliberately drops it (per the type comment); the
  // dedicated endpoint is the right one to read from. Stays cheap because
  // there is at most one Session worth fetching per conversation.
  const proxyTargetsQuery = useQuery({
    queryKey: ["proxyTargets", activeSessionId],
    queryFn: () =>
      activeSessionId
        ? apiClient.listProxyTargets(activeSessionId)
        : Promise.resolve({ targets: [] }),
    enabled: !!activeSessionId,
    refetchOnMount: "always",
  });

  const isVisible = useDocumentVisible();
  const isIdle = conversationQuery.data?.status === "idle";
  const probesPaused = !isVisible || isIdle;

  const targets = useMemo(
    () => proxyTargetsQuery.data?.targets ?? [],
    [proxyTargetsQuery.data],
  );
  const probeStatuses = useTargetProbes(targets, conversationId, probesPaused);

  const targetsWithStatus = useMemo<ProxyTargetWithStatus[]>(() => {
    return targets.map((target) => {
      let status: ProxyStatus;
      if (isIdle) {
        status = "idle";
      } else {
        status = probeStatuses.get(target.port) ?? "starting";
      }
      return { ...target, status };
    });
  }, [targets, probeStatuses, isIdle]);

  return {
    targets: targetsWithStatus,
    sessionId: activeSessionId,
    isLoading:
      conversationQuery.isLoading ||
      sessionsQuery.isLoading ||
      proxyTargetsQuery.isLoading,
  };
}
